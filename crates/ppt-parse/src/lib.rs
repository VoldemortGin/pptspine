#![forbid(unsafe_code)]
//! `ppt-parse` —— pptspine 的 OOXML 读取层(本轮核心)。
//!
//! 把一个 `.pptx`(zip + XML)解析成 [`ParsedPptx`]:一个 [`Presentation`] 结构化模型,
//! 外加一份 `media` 字节表(`裸文件名 -> 原始图片字节`)。解析全程容错,失败收敛成 [`PptError`]。

pub mod resolve;
mod xml;
mod zip_pkg;

use std::collections::BTreeMap;
use std::path::Path;

use ppt_core::model::{Presentation, Shape, Slide};
use ppt_core::style::{TextStyleLevels, TxStyles};
use ppt_core::theme::{ClrMap, Theme};
use ppt_core::{PptError, Result};

use zip_pkg::Package;

pub use resolve::{resolve, resolve_parts};

/// 解析输出:结构化演示文稿 + media 字节(键为裸文件名,如 `image1.png`)
/// + 继承链部件(layout / master / theme,供 [`resolve`] 消费)。
#[derive(Debug, Clone)]
pub struct ParsedPptx {
    pub presentation: Presentation,
    pub media: BTreeMap<String, Vec<u8>>,
    pub inherit: InheritanceParts,
}

/// 继承链解析所需的部件 IR(键为裸部件名,如 `slideLayout1.xml`)。
#[derive(Debug, Clone, Default)]
pub struct InheritanceParts {
    pub layouts: BTreeMap<String, LayoutPart>,
    pub masters: BTreeMap<String, MasterPart>,
    pub themes: BTreeMap<String, Theme>,
    /// `presentation.xml` 的 `p:defaultTextStyle`(非占位符文本框的继承基底)。
    pub default_text_style: Option<TextStyleLevels>,
}

/// 一个已解析的 slideLayout 部件。
#[derive(Debug, Clone, Default)]
pub struct LayoutPart {
    /// spTree 形状(占位符携带 `ph` / `lstStyle` / `xfrm`,供匹配与合并)。
    pub shapes: Vec<Shape>,
    /// `p:clrMapOvr > a:overrideClrMapping`;`None` = 沿用 master 映射。
    pub clr_map_ovr: Option<ClrMap>,
    /// 所属母版裸名(经 layout rels)。
    pub master_name: Option<String>,
}

/// 一个已解析的 slideMaster 部件。
#[derive(Debug, Clone, Default)]
pub struct MasterPart {
    pub shapes: Vec<Shape>,
    /// `p:clrMap`(master 必有;缺失时解析为 `None`,消费方按惯例缺省)。
    pub clr_map: Option<ClrMap>,
    /// `p:txStyles` 三桶(title / body / other)。
    pub tx_styles: Option<TxStyles>,
    /// 关联主题裸名(经 master rels)。
    pub theme_name: Option<String>,
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
        let data = xml::slide::parse_part(&slide_xml, rels_xml.as_deref(), &media_index);

        let layout_name = pkg.layout_name_for(part);
        let master_name = layout_name
            .as_deref()
            .and_then(|ln| pkg.master_name_for_layout(ln));

        // 演讲者备注:经 slide 的 .rels 找到 notesSlide 部件,提取其 body 占位符文字。
        let notes = rels_xml
            .as_deref()
            .and_then(|r| xml::first_rel_target_with(r, "notesSlide"))
            .and_then(|t| pkg.part_str(&t))
            .and_then(|nx| xml::notes::parse(&nx));

        slides.push(Slide {
            index,
            shapes: data.shapes,
            layout_name,
            master_name,
            notes,
            clr_map_ovr: data.clr_map_ovr,
            background: data.background,
        });
    }

    if slides.is_empty() && !ordered_parts.is_empty() {
        // 有 slide 部件却一张都没解析成功 —— 视为结构异常。
        return Err(PptError::Xml("no slides could be parsed".into()));
    }

    // 6) 继承链部件:slide 引用的 layout -> master -> theme(按裸名去重,B-8/B-9)。
    let inherit = collect_inheritance(&pkg, &slides, meta.default_text_style);

    Ok(ParsedPptx {
        presentation: Presentation {
            slides,
            slide_size: meta.slide_size,
        },
        media,
        inherit,
    })
}

/// 解析各 slide 引用到的 layout / master / theme 部件(去重;容错:缺失部件跳过)。
fn collect_inheritance(
    pkg: &Package,
    slides: &[Slide],
    default_text_style: Option<TextStyleLevels>,
) -> InheritanceParts {
    let empty_media = BTreeMap::new();
    let mut inherit = InheritanceParts {
        default_text_style,
        ..InheritanceParts::default()
    };

    for slide in slides {
        let Some(layout_name) = slide.layout_name.as_deref() else {
            continue;
        };
        if !inherit.layouts.contains_key(layout_name) {
            if let Some(xml_text) = pkg.layout_part_str(layout_name) {
                let data = xml::slide::parse_part(&xml_text, None, &empty_media);
                inherit.layouts.insert(
                    layout_name.to_string(),
                    LayoutPart {
                        shapes: data.shapes,
                        clr_map_ovr: data.clr_map_ovr,
                        master_name: pkg.master_name_for_layout(layout_name),
                    },
                );
            }
        }
        let Some(master_name) = inherit
            .layouts
            .get(layout_name)
            .and_then(|l| l.master_name.clone())
        else {
            continue;
        };
        if !inherit.masters.contains_key(&master_name) {
            if let Some(xml_text) = pkg.master_part_str(&master_name) {
                let data = xml::slide::parse_part(&xml_text, None, &empty_media);
                inherit.masters.insert(
                    master_name.clone(),
                    MasterPart {
                        shapes: data.shapes,
                        clr_map: data.clr_map,
                        tx_styles: data.tx_styles,
                        theme_name: pkg.theme_name_for_master(&master_name),
                    },
                );
            }
        }
        let Some(theme_name) = inherit
            .masters
            .get(&master_name)
            .and_then(|m| m.theme_name.clone())
        else {
            continue;
        };
        if let std::collections::btree_map::Entry::Vacant(slot) = inherit.themes.entry(theme_name) {
            if let Some(xml_text) = pkg.theme_part_str(slot.key()) {
                slot.insert(xml::theme::parse(&xml_text));
            }
        }
    }
    inherit
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
