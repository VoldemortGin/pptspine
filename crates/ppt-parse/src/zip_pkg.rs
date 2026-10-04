//! pptx zip 容器读取。
//!
//! `.pptx` = OOXML = 一个 zip 包。这里把整个包**一次性读进内存**(演示文稿通常不大),
//! 然后按名取用各 XML 部件与 media 字节。容器层失败收敛成 [`PptError::Zip`];
//! 资源限额([`ZipLimits`])命中收敛成 [`PptError::LimitExceeded`]。

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use ppt_core::{LimitKind, PptError, Result};
use zip::ZipArchive;

use crate::links::resolve_part_path;

/// 读取 zip 包时的资源限额(防 zip 炸弹 / 伪造头字段 / 恶意条目名)。
///
/// 任何一项超限都返回 [`PptError::LimitExceeded`],绝不 panic、绝不按声明值巨量预分配。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipLimits {
    /// 最大条目数(含目录条目)。
    pub max_entries: usize,
    /// 单个条目的最大解压字节数(先比声明值,再以实际读出量兜底)。
    pub max_entry_bytes: u64,
    /// 全部条目的累计实际解压字节上限。
    pub max_total_bytes: u64,
    /// 最大压缩比(解压 / 压缩);仅当解压量 > 1 MiB 时才判定(避免误伤小文件)。
    ///
    /// 默认 10 000:deflate 对大块零的理论上限约 1032:1,若设 1000 会误拒超过 1 MiB 的
    /// 合法纯色位图等 media。内存风险已由单条目 / 总量上限兜住,压缩比检查只用于拦
    /// 非 deflate 方法(bzip2 / zstd / lzma)的极端比值。
    pub max_compression_ratio: u32,
    /// 条目名最大字节长度。
    pub max_name_len: usize,
    /// 最大幻灯片数(`p:sldIdLst` 去重后的引用数)。
    ///
    /// 默认 5 000:现实中最大的演示文稿也只有数百到两三千页,5 000 留足 2 倍以上余量;
    /// 同时它严格小于默认 `max_entries`(10 000),使页数上限在条目数上限之前生效。
    /// 重复引用同一部件只算一次(见 `resolve_slide_order`)。
    pub max_slides: usize,
}

/// 压缩比检查的起判门槛:解压量不超过 1 MiB 的条目不做压缩比判定(避免误伤小文件)。
const RATIO_MIN_BYTES: u64 = 1024 * 1024;

impl Default for ZipLimits {
    fn default() -> Self {
        ZipLimits {
            max_entries: 10_000,
            max_entry_bytes: 256 * 1024 * 1024,
            max_total_bytes: 1024 * 1024 * 1024,
            max_compression_ratio: 10_000,
            max_name_len: 1024,
            max_slides: 5_000,
        }
    }
}

fn limit_err(kind: LimitKind, limit: u64, actual: u64) -> PptError {
    PptError::LimitExceeded {
        kind,
        limit,
        actual,
    }
}

/// `uncompressed / compressed > max_ratio` 且 `uncompressed > RATIO_MIN_BYTES` 时报错。
fn check_ratio(uncompressed: u64, compressed: u64, limits: &ZipLimits) -> Result<()> {
    if uncompressed <= RATIO_MIN_BYTES {
        return Ok(());
    }
    let max = u128::from(limits.max_compression_ratio);
    if u128::from(uncompressed) > max * u128::from(compressed) {
        // 向上取整,保证 actual > limit。
        let actual = if compressed == 0 {
            u64::MAX
        } else {
            uncompressed.div_ceil(compressed)
        };
        return Err(limit_err(
            LimitKind::CompressionRatio,
            u64::from(limits.max_compression_ratio),
            actual,
        ));
    }
    Ok(())
}

/// 条目名是否安全:非绝对路径、无盘符前缀(`C:\x` / `C:/x`)、无 `..` 段、无 NUL
/// (复用 zip 的 `enclosed_name` 再加严)。
fn is_safe_name(file: &zip::read::ZipFile<'_>) -> bool {
    let name = file.name();
    let b = name.as_bytes();
    let drive_letter = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    !drive_letter
        && file.enclosed_name().is_some()
        && !name.split(['/', '\\']).any(|seg| seg == "..")
}

/// 解包后的 pptx 原始部件集合(尚未解析 XML)。
pub struct Package {
    /// 部件路径 -> 原始字节(如 `ppt/slides/slide1.xml`)。包含 XML 与 media。
    parts: BTreeMap<String, Vec<u8>>,
    /// 演示文稿主部件路径(经包根 `_rels/.rels` 的 `officeDocument` 关系定位,
    /// 缺失 / 畸形 / 指向不存在部件时回退 [`DEFAULT_MAIN_PART`])。
    main_part: String,
    /// 主部件所在目录(含末尾 `/`,主部件在包根时为空串):`slides/`、`media/`、
    /// `slideLayouts/` 等兄弟目录都挂在它下面,与 `ppt/` 目录同构。
    root: String,
}

/// 缺省主部件路径(`.rels` 缺失 / 畸形 / 无 officeDocument 关系时沿用)。
const DEFAULT_MAIN_PART: &str = "ppt/presentation.xml";

/// 经包根 `_rels/.rels` 定位主部件:取类型以 `/officeDocument` 结尾的关系,Target 按包根规范化
/// (越出包根的 `..` 被拒绝),且部件必须真的存在;否则回退缺省路径。
fn locate_main_part(parts: &BTreeMap<String, Vec<u8>>) -> String {
    parts
        .get("_rels/.rels")
        .map(|b| crate::xml::parse_rels(&String::from_utf8_lossy(b)))
        .and_then(|rels| {
            rels.values()
                .filter(|r| r.rel_type.ends_with("/officeDocument"))
                .map(|r| resolve_part_path("", &r.target))
                .find(|p| parts.contains_key(p))
        })
        .unwrap_or_else(|| DEFAULT_MAIN_PART.to_string())
}

impl Package {
    /// 从内存字节打开一个 pptx 包,在 `limits` 约束下读出全部条目。
    pub fn open_bytes_with_limits(bytes: &[u8], limits: &ZipLimits) -> Result<Package> {
        let reader = Cursor::new(bytes);
        let mut archive =
            ZipArchive::new(reader).map_err(|e| PptError::Zip(format!("open archive: {e}")))?;
        if archive.len() > limits.max_entries {
            return Err(limit_err(
                LimitKind::Entries,
                limits.max_entries as u64,
                archive.len() as u64,
            ));
        }
        let mut parts = BTreeMap::new();
        let mut total: u64 = 0;
        for i in 0..archive.len() {
            let file = archive
                .by_index(i)
                .map_err(|e| PptError::Zip(format!("entry {i}: {e}")))?;
            let name_len = file.name_raw().len();
            if name_len > limits.max_name_len {
                return Err(limit_err(
                    LimitKind::NameLength,
                    limits.max_name_len as u64,
                    name_len as u64,
                ));
            }
            if !is_safe_name(&file) {
                return Err(PptError::Zip(format!(
                    "unsafe entry path: {:?}",
                    file.name()
                )));
            }
            // 跳过目录条目。
            if file.is_dir() {
                continue;
            }
            // 用 zip 规范化的名字(始终是 `/` 分隔)。
            let name = file.name().to_string();
            let compressed = file.compressed_size();
            let declared = file.size();
            if declared > limits.max_entry_bytes {
                return Err(limit_err(
                    LimitKind::EntryBytes,
                    limits.max_entry_bytes,
                    declared,
                ));
            }
            check_ratio(declared, compressed, limits)?;

            // 不信任声明大小:不按它预分配,按"本条目 / 总量剩余"二者较小者 +1 截断读取,
            // 读满即判超限(声明值造假也逃不掉)。
            let remaining_total = limits.max_total_bytes.saturating_sub(total);
            let cap = limits.max_entry_bytes.min(remaining_total);
            let mut buf = Vec::new();
            file.take(cap.saturating_add(1))
                .read_to_end(&mut buf)
                .map_err(|e| PptError::Zip(format!("read {name}: {e}")))?;
            let actual = buf.len() as u64;
            if actual > cap {
                // TotalBytes 报累计值(截断时 = max_total_bytes + 1,必 > limit)。
                return Err(if actual > limits.max_entry_bytes {
                    limit_err(LimitKind::EntryBytes, limits.max_entry_bytes, actual)
                } else {
                    limit_err(
                        LimitKind::TotalBytes,
                        limits.max_total_bytes,
                        total + actual,
                    )
                });
            }
            check_ratio(actual, compressed, limits)?;
            total += actual;
            parts.insert(name, buf);
        }
        let main_part = locate_main_part(&parts);
        let root = main_part
            .rsplit_once('/')
            .map_or(String::new(), |(dir, _)| format!("{dir}/"));
        Ok(Package {
            parts,
            main_part,
            root,
        })
    }

    /// 主部件所在目录前缀(含末尾 `/`;缺省布局下为 `ppt/`)。
    pub fn root(&self) -> &str {
        &self.root
    }

    /// 主部件路径。
    pub fn main_part(&self) -> &str {
        &self.main_part
    }

    /// 版式部件路径(`layout_name` 是裸名如 `slideLayout1.xml`)。
    pub fn layout_path(&self, layout_name: &str) -> String {
        format!("{}slideLayouts/{layout_name}", self.root)
    }

    /// 母版部件路径(裸名如 `slideMaster1.xml`)。
    pub fn master_path(&self, master_name: &str) -> String {
        format!("{}slideMasters/{master_name}", self.root)
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

    /// 演示文稿主部件(缺省 `ppt/presentation.xml`)的文本(必有,缺失即非法 pptx)。
    pub fn presentation_xml(&self) -> Result<String> {
        self.part_str(&self.main_part)
            .ok_or_else(|| PptError::Zip(format!("missing {}", self.main_part)))
    }

    /// 所有幻灯片部件名,按 `slideN` 的数字 N 升序。
    ///
    /// 注意:这只是一个**确定性的兜底排序**;真正的呈现顺序由 `presentation.xml` 的
    /// `p:sldId` + 关系决定(见 [`Self::slide_part_for_rid`])。
    pub fn slide_names_sorted(&self) -> Vec<String> {
        let prefix = format!("{}slides/slide", self.root);
        let mut names: Vec<String> = self
            .parts
            .keys()
            .filter(|k| k.starts_with(&prefix) && k.ends_with(".xml") && !k.contains("/_rels/"))
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

    /// 主部件的 `.rels` 文本(把 `p:sldId@r:id` 映射到具体 slide 部件;缺省
    /// `ppt/_rels/presentation.xml.rels`)。
    pub fn presentation_rels_str(&self) -> Option<String> {
        self.part_str(&rels_path_for(&self.main_part))
    }

    /// 取一张 media 图片的原始字节(`target` 形如 `ppt/media/image1.png`)。
    #[allow(dead_code)] // 保留为完整 media 访问 API,暂未被内部消费
    pub fn media_bytes(&self, target: &str) -> Option<&[u8]> {
        self.part(target)
    }

    /// 收集全部 `<root>media/*` 字节(缺省 `ppt/media/`),键为**裸文件名**(如 `image1.png`)。
    pub fn collect_media(&self) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let prefix = format!("{}media/", self.root);
        for (k, v) in &self.parts {
            if let Some(rest) = k.strip_prefix(prefix.as_str()) {
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
        let target = crate::xml::first_rel_target_with(&rels, slide_part, "slideLayout")?;
        Some(basename(&target))
    }

    /// 一个版式关联的母版名(best-effort)。
    pub fn master_name_for_layout(&self, layout_name: &str) -> Option<String> {
        // layout_name 是裸名如 `slideLayout1.xml`;其 rels 在
        // `ppt/slideLayouts/_rels/slideLayout1.xml.rels`。
        let layout_part = self.layout_path(layout_name);
        let rels = self.part_str(&rels_path_for(&layout_part))?;
        let target = crate::xml::first_rel_target_with(&rels, &layout_part, "slideMaster")?;
        Some(basename(&target))
    }

    /// 一个母版关联的主题名(best-effort,经母版 rels 的 `theme` 关系)。
    pub fn theme_name_for_master(&self, master_name: &str) -> Option<String> {
        let master_part = self.master_path(master_name);
        let rels = self.part_str(&rels_path_for(&master_part))?;
        let target = crate::xml::first_rel_target_with(&rels, &master_part, "theme")?;
        Some(basename(&target))
    }

    /// 版式部件文本(`layout_name` 是裸名如 `slideLayout1.xml`)。
    pub fn layout_part_str(&self, layout_name: &str) -> Option<String> {
        self.part_str(&self.layout_path(layout_name))
    }

    /// 母版部件文本(裸名如 `slideMaster1.xml`)。
    pub fn master_part_str(&self, master_name: &str) -> Option<String> {
        self.part_str(&self.master_path(master_name))
    }

    /// 主题部件文本(裸名如 `theme1.xml`)。
    pub fn theme_part_str(&self, theme_name: &str) -> Option<String> {
        self.part_str(&format!("{}theme/{theme_name}", self.root))
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
