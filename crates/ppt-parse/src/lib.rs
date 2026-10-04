#![forbid(unsafe_code)]
//! `ppt-parse` —— pptspine 的 OOXML 读取层(本轮核心)。
//!
//! 把一个 `.pptx`(zip + XML)解析成 [`ParsedPptx`]:一个 [`Presentation`] 结构化模型,
//! 外加一份 `media` 字节表(`裸文件名 -> 原始图片字节`)。解析全程容错,失败收敛成 [`PptError`]。

mod charts;
mod diagrams;
mod links;
pub mod resolve;
mod xml;
mod zip_pkg;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use ppt_core::model::{
    Background, Comment, DocProperties, Presentation, Section, Shape, Slide, TableStyle,
};
use ppt_core::style::{TextStyleLevels, TxStyles};
use ppt_core::theme::{ClrMap, Theme};
use ppt_core::{DiagnosticKind, PptError, Result};

use zip_pkg::Package;

pub use ppt_core::LimitKind;
pub use resolve::{resolve, resolve_parts};
pub use zip_pkg::ZipLimits;

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
    /// `ppt/tableStyles.xml` 的表格样式(键为 `a:tblStyle@styleId`);部件缺失 / 畸形时为空。
    pub table_styles: BTreeMap<String, TableStyle>,
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
    /// `p:cSld > p:bg`(B-10 继承链:slide 无 bg 时回退到此)。
    pub background: Option<Background>,
    /// `p:sldLayout@showMasterSp`;`None` = 缺省(显示 master 非占位符形状)。
    pub show_master_sp: Option<bool>,
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
    /// `p:cSld > p:bg`(B-10 继承链:slide / layout 皆无 bg 时回退到此)。
    pub background: Option<Background>,
}

/// 从磁盘路径解析一个 `.pptx`。
pub fn parse_path(path: &Path) -> Result<ParsedPptx> {
    parse_path_with_limits(path, &ZipLimits::default())
}

/// 同 [`parse_path`],但使用调用方给定的 zip 读取限额。
pub fn parse_path_with_limits(path: &Path, limits: &ZipLimits) -> Result<ParsedPptx> {
    let bytes = std::fs::read(path)?;
    parse_bytes_with_limits(&bytes, limits)
}

/// 从内存字节解析一个 `.pptx`(默认限额 [`ZipLimits::default`])。
pub fn parse_bytes(bytes: &[u8]) -> Result<ParsedPptx> {
    parse_bytes_with_limits(bytes, &ZipLimits::default())
}

/// 同 [`parse_bytes`],但使用调用方给定的 zip 读取限额;超限返回
/// [`PptError::LimitExceeded`]。
pub fn parse_bytes_with_limits(bytes: &[u8], limits: &ZipLimits) -> Result<ParsedPptx> {
    let pkg = Package::open_bytes_with_limits(bytes, limits)?;

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
    let ordered_parts = resolve_slide_order(&meta.slide_rids, &pres_rels, &pkg, limits)?;

    let comment_authors = collect_comment_authors(&pkg, &pres_rels);
    let mut comment_cache = CommentCache::new(limits);

    // 5) 逐张解析 slide(`slide_parts[i]` = `slides[i]` 的部件路径与 rels,供链接后处理)。
    let mut slides = Vec::with_capacity(ordered_parts.len());
    let mut slide_parts: Vec<(&str, BTreeMap<String, xml::Relationship>)> = Vec::new();
    for (index, part) in ordered_parts.iter().enumerate() {
        let Some(slide_xml) = pkg.part_str(part) else {
            continue;
        };
        let rels_xml = pkg.slide_rels_str(part);
        let data = parse_shape_part(&pkg, part, &slide_xml, rels_xml.as_deref(), &media_index);
        slide_parts.push((
            part.as_str(),
            rels_xml.as_deref().map(xml::parse_rels).unwrap_or_default(),
        ));

        let layout_name = pkg.layout_name_for(part);
        let master_name = layout_name
            .as_deref()
            .and_then(|ln| pkg.master_name_for_layout(ln));

        // 演讲者备注:经 slide 的 .rels 找到 notesSlide 部件,提取其 body 占位符文字。
        let notes = rels_xml
            .as_deref()
            .and_then(|r| xml::first_rel_target_with(r, part, "notesSlide"))
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
            hidden: data.hidden,
            show_master_sp: data.show_master_sp.unwrap_or(true),
            comments: collect_comments(
                &pkg,
                part,
                rels_xml.as_deref(),
                &comment_authors,
                &mut comment_cache,
            ),
        });
    }

    // 5b) 超链接后处理:外链目标 + 内部跳转目标序号(需全量"部件 → 序号"映射);
    //     图表占位经 rels 读图表部件回填缓存数据。
    let part_index: BTreeMap<String, usize> = slide_parts
        .iter()
        .enumerate()
        .map(|(i, (part, _))| ((*part).to_string(), i))
        .collect();
    let count = slides.len();
    let mut chart_cache = charts::ChartCache::new(limits);
    let mut diagram_cache = diagrams::DiagramCache::new(limits);
    for (i, (slide, (part, rels))) in slides.iter_mut().zip(&slide_parts).enumerate() {
        let ctx = links::LinkCtx {
            rels,
            part,
            part_index: &part_index,
            current: i,
            count,
        };
        links::resolve_links(&mut slide.shapes, &ctx);
        charts::resolve_charts(&mut slide.shapes, rels, part, &pkg, &mut chart_cache);
        diagrams::resolve_diagrams(
            &mut slide.shapes,
            rels,
            part,
            &pkg,
            &media_index,
            &mut diagram_cache,
        );
    }

    if slides.is_empty() && !ordered_parts.is_empty() {
        // 有 slide 部件却一张都没解析成功 —— 视为结构异常。
        return Err(PptError::Xml("no slides could be parsed".into()));
    }

    // 6) 继承链部件:slide 引用的 layout -> master -> theme(按裸名去重,B-8/B-9)。
    let mut inherit = collect_inheritance(&pkg, &slides, meta.default_text_style, &media_index);
    inherit.table_styles = collect_table_styles(&pkg, &pres_rels);

    // 7) 节(`sldId@id` → 幻灯片序号)与文档属性。
    let sections = resolve_sections(
        &meta.sections,
        &meta.slide_ids,
        &pres_rels,
        &part_index,
        pkg.main_part(),
    );
    let properties = collect_doc_props(&pkg);

    Ok(ParsedPptx {
        presentation: Presentation {
            slides,
            slide_size: meta.slide_size,
            sections,
            properties,
            first_slide_num: meta.first_slide_num.unwrap_or(1),
            diagnostics: pkg.take_diagnostics(),
        },
        media,
        inherit,
    })
}

/// 解析一个形部件(slide / layout / master / SmartArt drawing),并把"嵌套超限被跳过的子树数"
/// 记进解析诊断(`part` 为该部件路径)。
pub(crate) fn parse_shape_part(
    pkg: &Package,
    part: &str,
    xml_text: &str,
    rels_xml: Option<&str>,
    media_index: &BTreeMap<String, usize>,
) -> xml::slide::PartData {
    let data = xml::slide::parse_part(xml_text, rels_xml, media_index);
    if data.nesting_skipped > 0 {
        pkg.note(DiagnosticKind::NestingTooDeep, part, data.nesting_skipped);
    }
    if data.custgeom_degraded > 0 {
        pkg.note(
            DiagnosticKind::CustomGeometryDegraded,
            part,
            data.custgeom_degraded,
        );
    }
    data
}

/// 解析各 slide 引用到的 layout / master / theme 部件(去重;容错:缺失部件跳过)。
fn collect_inheritance(
    pkg: &Package,
    slides: &[Slide],
    default_text_style: Option<TextStyleLevels>,
    media_index: &BTreeMap<String, usize>,
) -> InheritanceParts {
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
                // 经部件自身 rels 解析图片 `r:embed`(版式上的 logo / 图片背景)。
                let rels = pkg.slide_rels_str(&pkg.layout_path(layout_name));
                let data = parse_shape_part(
                    pkg,
                    &pkg.layout_path(layout_name),
                    &xml_text,
                    rels.as_deref(),
                    media_index,
                );
                inherit.layouts.insert(
                    layout_name.to_string(),
                    LayoutPart {
                        shapes: data.shapes,
                        clr_map_ovr: data.clr_map_ovr,
                        master_name: pkg.master_name_for_layout(layout_name),
                        background: data.background,
                        show_master_sp: data.show_master_sp,
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
                let rels = pkg.slide_rels_str(&pkg.master_path(&master_name));
                let data = parse_shape_part(
                    pkg,
                    &pkg.master_path(&master_name),
                    &xml_text,
                    rels.as_deref(),
                    media_index,
                );
                inherit.masters.insert(
                    master_name.clone(),
                    MasterPart {
                        shapes: data.shapes,
                        clr_map: data.clr_map,
                        tx_styles: data.tx_styles,
                        theme_name: pkg.theme_name_for_master(&master_name),
                        background: data.background,
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

/// 批注作者表:经 presentation rels 的 `commentAuthors`(旧式)/ `authors`(新式线程批注)
/// 关系定位(无关系时回退惯例路径);部件缺失 / 畸形 → 空表(批注作者为 `None`)。
fn collect_comment_authors(
    pkg: &Package,
    pres_rels: &BTreeMap<String, xml::Relationship>,
) -> BTreeMap<String, xml::comments::Author> {
    let mut parts: Vec<String> = pres_rels
        .values()
        .filter(|r| r.rel_type.ends_with("/commentAuthors") || r.rel_type.ends_with("/authors"))
        .map(|r| links::resolve_part_path(pkg.main_part(), &r.target))
        .collect();
    if parts.is_empty() {
        let root = pkg.root();
        parts = vec![
            format!("{root}commentAuthors.xml"),
            format!("{root}authors.xml"),
        ];
    }
    let mut map = BTreeMap::new();
    for part in parts {
        if let Some(x) = pkg.part_str(&part) {
            map.extend(xml::comments::parse_authors(&x));
        }
    }
    map
}

/// 批注解析缓存:键为批注部件路径,值为 `(解析结果, 因上限提前停止)`。多张幻灯片共享同一部件时
/// 只解析一次;每张幻灯片仍各拿一份拷贝,所以拷贝的条数(含回复)从全局预算
/// [`ZipLimits::max_comments`] 里扣,缓存只省解析、不省拷贝。
struct CommentCache {
    parsed: BTreeMap<String, Option<(Vec<Comment>, bool)>>,
    left: usize,
    max: usize,
}

impl CommentCache {
    fn new(limits: &ZipLimits) -> Self {
        CommentCache {
            parsed: BTreeMap::new(),
            left: limits.max_comments,
            max: limits.max_comments,
        }
    }
}

/// 从 `src` 里按预算 `budget`(批注 + 回复合计)拷贝前缀;返回拷贝结果。
fn take_comments(src: &[Comment], budget: &mut usize) -> Vec<Comment> {
    let mut out = Vec::new();
    for c in src {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let take = c.replies.len().min(*budget);
        *budget -= take;
        let mut copy = c.clone();
        copy.replies.truncate(take);
        out.push(copy);
    }
    out
}

/// 一张 slide 的批注:其 rels 里所有 `comments` 关系(旧式 / 新式)指向的部件,按 rId 序拼接。
/// 同一张 slide 内指向同一部件的多条关系只取首条(其余记 [`DiagnosticKind::DuplicateCommentRef`]);
/// 整个演示文稿的批注总条数(含回复)受 [`ZipLimits::max_comments`] 约束,超出截断并记
/// [`DiagnosticKind::CommentsTruncated`]。
fn collect_comments(
    pkg: &Package,
    slide_part: &str,
    slide_rels_xml: Option<&str>,
    authors: &BTreeMap<String, xml::comments::Author>,
    cache: &mut CommentCache,
) -> Vec<Comment> {
    let rels = slide_rels_xml.map(xml::parse_rels).unwrap_or_default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for r in rels.values().filter(|r| r.rel_type.ends_with("/comments")) {
        let path = links::resolve_part_path(slide_part, &r.target);
        if !seen.insert(path.clone()) {
            // 只对真实存在的部件记(指向缺失部件的重复引用已由 `missing-part` 覆盖)。
            if pkg.has_part(&path) {
                pkg.note(DiagnosticKind::DuplicateCommentRef, &path, 1);
            }
            continue;
        }
        let max = cache.max;
        let cached = cache
            .parsed
            .entry(path.clone())
            .or_insert_with(|| {
                pkg.part_str(&path).map(|x| {
                    let p = xml::comments::parse_comments(&x, authors, max);
                    // 批注正文里的嵌套超限只在这里能看到,每个部件只记一次。
                    if p.nesting_skipped > 0 {
                        pkg.note(DiagnosticKind::NestingTooDeep, &path, p.nesting_skipped);
                    }
                    (p.comments, p.truncated)
                })
            })
            .as_ref();
        let Some((comments, stopped)) = cached else {
            continue;
        };
        let want = xml::comments::count_comments(comments);
        let before = cache.left;
        out.extend(take_comments(comments, &mut cache.left));
        if *stopped || before < want {
            pkg.note(DiagnosticKind::CommentsTruncated, &path, 1);
        }
    }
    out
}

/// 表格样式部件:经 presentation rels 的 `tableStyles` 关系定位(缺失回退到惯例路径
/// `ppt/tableStyles.xml`);部件缺失 / 畸形时返回空表(表格退回只用显式属性)。
fn collect_table_styles(
    pkg: &Package,
    pres_rels: &BTreeMap<String, xml::Relationship>,
) -> BTreeMap<String, TableStyle> {
    let part = pres_rels
        .values()
        .find(|r| r.rel_type.ends_with("/tableStyles"))
        .map(|r| links::resolve_part_path(pkg.main_part(), &r.target))
        .unwrap_or_else(|| format!("{}tableStyles.xml", pkg.root()));
    pkg.part_str(&part)
        .map(|x| xml::table_style::parse(&x))
        .unwrap_or_default()
}

/// 节的 `sldId@id` 列表 → 幻灯片序号(经 `@id → r:id → 部件 → 序号`;解析不出的 id 丢弃)。
fn resolve_sections(
    sections: &[(String, Vec<u32>)],
    slide_ids: &[(u32, String)],
    pres_rels: &BTreeMap<String, xml::Relationship>,
    part_index: &BTreeMap<String, usize>,
    main_part: &str,
) -> Vec<Section> {
    let id_to_index: BTreeMap<u32, usize> = slide_ids
        .iter()
        .filter_map(|(id, rid)| {
            let target = links::resolve_part_path(main_part, &pres_rels.get(rid)?.target);
            Some((*id, *part_index.get(&target)?))
        })
        .collect();
    sections
        .iter()
        .map(|(name, ids)| Section {
            name: name.clone(),
            slide_indices: ids
                .iter()
                .filter_map(|id| id_to_index.get(id).copied())
                .collect(),
        })
        .collect()
}

/// 文档属性:经包根 rels 找 core / extended properties 部件(缺失回退到惯例路径
/// `docProps/core.xml` / `docProps/app.xml`);部件缺失则对应字段全 `None`。
fn collect_doc_props(pkg: &Package) -> DocProperties {
    let root_rels = pkg
        .part_str("_rels/.rels")
        .map(|s| xml::parse_rels(&s))
        .unwrap_or_default();
    let target_of = |suffix: &str, fallback: &str| {
        root_rels
            .values()
            .find(|r| r.rel_type.ends_with(suffix))
            .map(|r| r.target.trim_start_matches('/').to_string())
            .unwrap_or_else(|| fallback.to_string())
    };
    let mut props = DocProperties::default();
    if let Some(core) = pkg.part_str(&target_of("/core-properties", "docProps/core.xml")) {
        xml::doc_props::parse_core(&core, &mut props);
    }
    if let Some(app) = pkg.part_str(&target_of("/extended-properties", "docProps/app.xml")) {
        xml::doc_props::parse_app(&app, &mut props);
    }
    props
}

/// 把 presentation.xml 的 `r:id` 顺序解析成具体 slide 部件路径列表。
/// 拿不到关系映射时,回退到按 `slideN` 数字升序(确定性兜底)。
/// 同一部件被 `p:sldIdLst` 重复引用只保留首次出现(保持顺序):合法文件里一个 slide 部件
/// 只会被引用一次,不去重则一个很小的文件就能把同一页放大成 N 份解析与存储。去重后的数量
/// 超过 [`ZipLimits::max_slides`] 返回 [`PptError::LimitExceeded`]。
fn resolve_slide_order(
    rids: &[String],
    pres_rels: &BTreeMap<String, xml::Relationship>,
    pkg: &Package,
    limits: &ZipLimits,
) -> Result<Vec<String>> {
    let mut parts: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for rid in rids {
        if let Some(rel) = pres_rels.get(rid) {
            let target = links::resolve_part_path(pkg.main_part(), &rel.target);
            if seen.contains(&target) {
                // 重复引用只保留首次;被去掉的份数记诊断(`part` = 被重复引用的 slide 部件)。
                pkg.note(DiagnosticKind::DuplicateSlideRef, &target, 1);
            } else if pkg.part_str(&target).is_some() {
                seen.insert(target.clone());
                parts.push(target);
            }
        }
    }
    if parts.is_empty() {
        // 兜底:直接按 slide 文件名数字序。
        parts = pkg.slide_names_sorted();
    }
    if parts.len() > limits.max_slides {
        return Err(PptError::LimitExceeded {
            kind: LimitKind::Slides,
            limit: limits.max_slides as u64,
            actual: parts.len() as u64,
        });
    }
    Ok(parts)
}
