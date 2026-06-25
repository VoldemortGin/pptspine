#![forbid(unsafe_code)]
//! `ppt-parse` —— pptspine 的 OOXML 读取层(本轮核心)。
//!
//! 把一个 `.pptx`(zip + XML)解析成 [`ParsedPptx`]:一个 [`Presentation`] 结构化模型,
//! 外加一份 `media` 字节表(`裸文件名 -> 原始图片字节`)。解析全程容错,失败收敛成 [`PptError`]。

mod zip_pkg;
mod xml;

use std::collections::BTreeMap;
use std::path::Path;

use ppt_core::model::{Presentation, Slide};
use ppt_core::{PptError, Result};

use zip_pkg::Package;

/// 解析输出:结构化演示文稿 + media 字节(键为裸文件名,如 `image1.png`)。
#[derive(Debug, Clone)]
pub struct ParsedPptx {
    pub presentation: Presentation,
    pub media: BTreeMap<String, Vec<u8>>,
}

/// 从磁盘路径解析一个 `.pptx`。
pub fn parse_path(path: &Path) -> Result<ParsedPptx> {
    let bytes = std::fs::read(path)?;
    parse_bytes(&bytes)
}

/// 从内存字节解析一个 `.pptx`。
pub fn parse_bytes(bytes: &[u8]) -> Result<ParsedPptx> {
    let pkg = Package::open_bytes(bytes)?;

    // 1) presentation.xml:画布尺寸 + 幻灯片顺序(r:id 列表)。
    let pres_xml = pkg.presentation_xml()?;
    let meta = xml::presentation::parse(&pres_xml);

    // 2) presentation 的 rels:把 r:id 映射到具体 slide 部件路径。
    let pres_rels = pkg
        .presentation_rels_str()
        .map(|s| xml::parse_rels(&s))
        .unwrap_or_default();

    // 3) media:一次性收集字节 + 建立长度索引(供 Picture.image_bytes_len 回填)。
    let media = pkg.collect_media();
    let media_index: BTreeMap<String, usize> =
        media.iter().map(|(k, v)| (k.clone(), v.len())).collect();

    // 4) 按 presentation.xml 的 r:id 顺序确定 slide 部件;拿不到关系时回退到 slideN 数字序。
    let ordered_parts = resolve_slide_order(&meta.slide_rids, &pres_rels, &pkg);

    // 5) 逐张解析 slide。
    let mut slides = Vec::with_capacity(ordered_parts.len());
    for (index, part) in ordered_parts.iter().enumerate() {
        let Some(slide_xml) = pkg.part_str(part) else {
            continue;
        };
        let rels_xml = pkg.slide_rels_str(part);
        let shapes = xml::slide::parse(&slide_xml, rels_xml.as_deref(), &media_index);

        let layout_name = pkg.layout_name_for(part);
        let master_name = layout_name
            .as_deref()
            .and_then(|ln| pkg.master_name_for_layout(ln));

        slides.push(Slide {
            index,
            shapes,
            layout_name,
            master_name,
        });
    }

    if slides.is_empty() && !ordered_parts.is_empty() {
        // 有 slide 部件却一张都没解析成功 —— 视为结构异常。
        return Err(PptError::Xml("no slides could be parsed".into()));
    }

    Ok(ParsedPptx {
        presentation: Presentation {
            slides,
            slide_size: meta.slide_size,
        },
        media,
    })
}

/// 把 presentation.xml 的 `r:id` 顺序解析成具体 slide 部件路径列表。
/// 拿不到关系映射时,回退到按 `slideN` 数字升序(确定性兜底)。
fn resolve_slide_order(
    rids: &[String],
    pres_rels: &BTreeMap<String, xml::Relationship>,
    pkg: &Package,
) -> Vec<String> {
    let mut parts = Vec::new();
    for rid in rids {
        if let Some(rel) = pres_rels.get(rid) {
            let target = xml::normalize_target(&rel.target);
            if pkg.part_str(&target).is_some() {
                parts.push(target);
            }
        }
    }
    if parts.is_empty() {
        // 兜底:直接按 slide 文件名数字序。
        parts = pkg.slide_names_sorted();
    }
    parts
}
