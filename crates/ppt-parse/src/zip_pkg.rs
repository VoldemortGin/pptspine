//! pptx zip 容器读取。
//!
//! `.pptx` = OOXML = 一个 zip 包。这里把整个包**一次性读进内存**(演示文稿通常不大),
//! 然后按名取用各 XML 部件与 media 字节。所有失败收敛成 [`PptError::Zip`]。

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use ppt_core::{PptError, Result};
use zip::ZipArchive;

/// 解包后的 pptx 原始部件集合(尚未解析 XML)。
pub struct Package {
    /// 部件路径 -> 原始字节(如 `ppt/slides/slide1.xml`)。包含 XML 与 media。
    parts: BTreeMap<String, Vec<u8>>,
}

impl Package {
    /// 从内存字节打开一个 pptx 包,读出全部条目。
    pub fn open_bytes(bytes: &[u8]) -> Result<Package> {
        let reader = Cursor::new(bytes);
        let mut archive =
            ZipArchive::new(reader).map_err(|e| PptError::Zip(format!("open archive: {e}")))?;
        let mut parts = BTreeMap::new();
        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| PptError::Zip(format!("entry {i}: {e}")))?;
            // 跳过目录条目。
            if file.is_dir() {
                continue;
            }
            // 用 zip 规范化的名字(始终是 `/` 分隔)。
            let name = file.name().to_string();
            let mut buf = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut buf)
                .map_err(|e| PptError::Zip(format!("read {name}: {e}")))?;
            parts.insert(name, buf);
        }
        Ok(Package { parts })
    }

    /// 取一个部件的字节(只读引用)。
    #[allow(dead_code)] // 保留为完整包访问 API,暂未被内部消费
    pub fn part(&self, name: &str) -> Option<&[u8]> {
        self.parts.get(name).map(|v| v.as_slice())
    }

    /// 取一个部件并解码为 UTF-8 字符串(XML 部件用)。
    pub fn part_str(&self, name: &str) -> Option<String> {
        self.parts
            .get(name)
            .map(|v| String::from_utf8_lossy(v).into_owned())
    }

    /// `ppt/presentation.xml` 的文本(必有,缺失即非法 pptx)。
    pub fn presentation_xml(&self) -> Result<String> {
        self.part_str("ppt/presentation.xml")
            .ok_or_else(|| PptError::Zip("missing ppt/presentation.xml".into()))
    }

    /// 所有幻灯片部件名,按 `slideN` 的数字 N 升序。
    ///
    /// 注意:这只是一个**确定性的兜底排序**;真正的呈现顺序由 `presentation.xml` 的
    /// `p:sldId` + 关系决定(见 [`Self::slide_part_for_rid`])。
    pub fn slide_names_sorted(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .parts
            .keys()
            .filter(|k| {
                k.starts_with("ppt/slides/slide") && k.ends_with(".xml") && !k.contains("/_rels/")
            })
            .cloned()
            .collect();
        names.sort_by_key(|n| slide_number(n).unwrap_or(u32::MAX));
        names
    }

    /// 给定一个幻灯片部件名,返回其 `.rels`(关系)部件文本(若存在)。
    ///
    /// 关系文件位于 `ppt/slides/_rels/slideN.xml.rels`。
    pub fn slide_rels_str(&self, slide_part: &str) -> Option<String> {
        let rels = rels_path_for(slide_part);
        self.part_str(&rels)
    }

    /// `ppt/_rels/presentation.xml.rels` 的文本(把 `p:sldId@r:id` 映射到具体 slide 部件)。
    pub fn presentation_rels_str(&self) -> Option<String> {
        self.part_str("ppt/_rels/presentation.xml.rels")
    }

    /// 取一张 media 图片的原始字节(`target` 形如 `ppt/media/image1.png`)。
    #[allow(dead_code)] // 保留为完整 media 访问 API,暂未被内部消费
    pub fn media_bytes(&self, target: &str) -> Option<&[u8]> {
        self.part(target)
    }

    /// 收集全部 `ppt/media/*` 字节,键为**裸文件名**(如 `image1.png`)。
    pub fn collect_media(&self) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        for (k, v) in &self.parts {
            if let Some(rest) = k.strip_prefix("ppt/media/") {
                if !rest.is_empty() && !rest.contains('/') {
                    out.insert(rest.to_string(), v.clone());
                }
            }
        }
        out
    }

    /// 一个幻灯片关联的版式名(best-effort):经 slide 的 `.rels` 找到 slideLayout 目标,
    /// 取其裸文件名(如 `slideLayout1.xml`)。失败返回 `None`。
    pub fn layout_name_for(&self, slide_part: &str) -> Option<String> {
        let rels = self.slide_rels_str(slide_part)?;
        let target = crate::xml::first_rel_target_with(&rels, "slideLayout")?;
        Some(basename(&target))
    }

    /// 一个版式关联的母版名(best-effort)。
    pub fn master_name_for_layout(&self, layout_name: &str) -> Option<String> {
        // layout_name 是裸名如 `slideLayout1.xml`;其 rels 在
        // `ppt/slideLayouts/_rels/slideLayout1.xml.rels`。
        let layout_part = format!("ppt/slideLayouts/{layout_name}");
        let rels = self.part_str(&rels_path_for(&layout_part))?;
        let target = crate::xml::first_rel_target_with(&rels, "slideMaster")?;
        Some(basename(&target))
    }
}

/// 从 `ppt/slides/slideN.xml` 抽出数字 N。
fn slide_number(part: &str) -> Option<u32> {
    let file = basename(part);
    let stem = file.strip_suffix(".xml")?;
    let digits = stem.strip_prefix("slide")?;
    digits.parse::<u32>().ok()
}

/// 给一个部件路径,推出其 `_rels/*.rels` 路径。
/// 例如 `ppt/slides/slide1.xml` -> `ppt/slides/_rels/slide1.xml.rels`。
fn rels_path_for(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// 取一个 `/` 分隔路径的最后一段(裸文件名)。同时把 `../` 前缀去掉。
fn basename(path: &str) -> String {
    let p = path.rsplit('/').next().unwrap_or(path);
    p.to_string()
}
