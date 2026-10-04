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
pub use resolve::{resolve, resolve_parts, resolve_parts_capped, MAX_INHERITED_SHAPES};
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

/// 解析结果的模型字节估算(幻灯片 + layout / master 形状;共享值只计一次,口径见
/// [`ppt_core::model_bytes`])。解析侧按实际产生的每个字符串 / 节点记账,所以对任何输入都有
/// `estimated_model_bytes(&parsed) <= limits.max_model_bytes`。
#[must_use]
pub fn estimated_model_bytes(p: &ParsedPptx) -> usize {
    let mut est = ppt_core::model_bytes::Estimator::new();
    let mut n = est.presentation(&p.presentation);
    for l in p.inherit.layouts.values() {
        n += est.shapes(&l.shapes);
    }
    for m in p.inherit.masters.values() {
        n += est.shapes(&m.shapes);
    }
    n
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

    // 1) presentation 的 rels:把 r:id 映射到具体 slide 部件路径。
    let pres_xml = pkg.presentation_xml()?;
    let pres_rels = pkg
        .presentation_rels_str()
        .map(|s| xml::parse_rels(&s))
        .unwrap_or_default();

    // 2) presentation.xml:画布尺寸 + 幻灯片顺序(`sldId` 边读边折成部件路径、边去重,
    //    收集到 `max_slides + 1` 个就停)。
    let meta = xml::presentation::parse(&pres_xml, limits.max_slides, |rid| {
        let rel = pres_rels.get(rid)?;
        let target = links::resolve_part_path(pkg.main_part(), &rel.target);
        pkg.has_part(&target).then_some(target)
    });

    // 3) media:一次性收集字节 + 建立长度索引(供 Picture.image_bytes_len 回填)。
    let media = pkg.collect_media();
    let media_index: BTreeMap<String, usize> =
        media.iter().map(|(k, v)| (k.clone(), v.len())).collect();

    // 4) 按 presentation.xml 的 r:id 顺序确定 slide 部件;拿不到关系时回退到 slideN 数字序。
    let ordered_parts = resolve_slide_order(&meta, &pkg, limits)?;

    // 每页一个 `Slide` 骨架总是保留(幻灯片数已受 `max_slides` 约束):先从模型字节预算里预留。
    pkg.reserve_model_bytes(ordered_parts.len() * std::mem::size_of::<Slide>());
    // 截断公平性:后面每张幻灯片的保底额度先预留出来(见 `Package::plan_slides`)。
    pkg.plan_slides(ordered_parts.len());

    // 5) 先解析被所有幻灯片依赖的部件:layout -> master -> theme(按裸名去重,B-8/B-9)与表格样式。
    //    它们在幻灯片之前解析,又只拿"预留完全部幻灯片保底之后"的额度——既不会被幻灯片饿死,
    //    也不会把幻灯片饿死。
    let layout_names: Vec<Option<String>> = ordered_parts
        .iter()
        .map(|part| pkg.layout_name_for(part))
        .collect();
    let mut inherit =
        collect_inheritance(&pkg, &layout_names, meta.default_text_style, &media_index);
    inherit.table_styles = collect_table_styles(&pkg, &pres_rels);

    let comment_authors = collect_comment_authors(&pkg, &pres_rels);
    let mut comment_cache = CommentCache::new(limits);
    let mut chart_cache = charts::ChartCache::new(limits);
    let mut diagram_cache = diagrams::DiagramCache::new(limits);
    let part_index: BTreeMap<String, usize> = ordered_parts
        .iter()
        .enumerate()
        .map(|(i, part)| (part.clone(), i))
        .collect();

    // 6) 逐张解析 slide,连同它自己的附属部件(超链接回填、图表、SmartArt、备注、批注)一起,
    //    在"为后面的幻灯片预留保底"之后的额度内完成。
    let mut slides = Vec::with_capacity(ordered_parts.len());
    for (index, part) in ordered_parts.iter().enumerate() {
        pkg.set_pending(ordered_parts.len() - index - 1);
        let Some(slide_xml) = pkg.part_str(part) else {
            continue;
        };
        let rels_xml = pkg.slide_rels_str(part);
        let data = parse_shape_part(&pkg, part, &slide_xml, rels_xml.as_deref(), &media_index);
        let rels = rels_xml.as_deref().map(xml::parse_rels).unwrap_or_default();
        let mut shapes = data.shapes;

        // 超链接:外链目标 + 内部跳转目标序号(需全量"部件 → 序号"映射);图表占位经 rels 读
        // 图表部件回填缓存数据;SmartArt 展开(在超链接之后:drawing 里的关系属于 drawing 部件)。
        let ctx = links::LinkCtx {
            rels: &rels,
            part,
            part_index: &part_index,
            current: index,
            count: ordered_parts.len(),
            pkg: &pkg,
            urls: Default::default(),
        };
        links::resolve_links(&mut shapes, &ctx);
        charts::resolve_charts(&mut shapes, &rels, part, &pkg, &mut chart_cache);
        diagrams::resolve_diagrams(
            &mut shapes,
            &rels,
            part,
            &pkg,
            &media_index,
            &mut diagram_cache,
        );

        let layout_name = layout_names[index].clone();
        let master_name = layout_name
            .as_deref()
            .and_then(|ln| pkg.master_name_for_layout(ln));

        // 演讲者备注:经 slide 的 .rels 找到 notesSlide 部件,提取其 body 占位符文字
        // (与其它部件同一套预算与诊断)。
        let notes = rels_xml
            .as_deref()
            .and_then(|r| xml::first_rel_target_with(r, part, "notesSlide"))
            .and_then(|t| {
                let nx = pkg.part_str(&t)?;
                pkg.budgeted(&t, |_| xml::notes::parse(&nx))
            });

        slides.push(Slide {
            index,
            shapes,
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
    pkg.set_pending(0);

    if slides.is_empty() && !ordered_parts.is_empty() {
        // 有 slide 部件却一张都没解析成功 —— 视为结构异常。
        return Err(PptError::Xml("no slides could be parsed".into()));
    }

    // 7) 节(`sldId@id` → 幻灯片序号)与文档属性。
    let sections = resolve_sections(&meta.sections, &meta.slide_ids, &part_index);
    let properties = collect_doc_props(&pkg);

    Ok(ParsedPptx {
        presentation: Presentation {
            slides,
            slide_size: meta.slide_size,
            sections,
            properties,
            first_slide_num: meta.first_slide_num.unwrap_or(1),
            diagnostics: pkg.take_diagnostics(),
            report: pkg.report(),
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
    let data = pkg.budgeted(part, |shapes| {
        xml::slide::parse_part(
            xml_text,
            rels_xml,
            media_index,
            xml::slide::PartBudget { shapes },
        )
    });
    pkg.spend_shapes(data.shapes_used);
    if data.shapes_dropped > 0 {
        pkg.note(DiagnosticKind::ShapesTruncated, part, data.shapes_dropped);
    }
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

/// 解析各 slide 引用到的 layout / master / theme 部件(按幻灯片顺序去重;容错:缺失部件跳过)。
fn collect_inheritance(
    pkg: &Package,
    layout_names: &[Option<String>],
    default_text_style: Option<TextStyleLevels>,
    media_index: &BTreeMap<String, usize>,
) -> InheritanceParts {
    let mut inherit = InheritanceParts {
        default_text_style,
        ..InheritanceParts::default()
    };

    for layout_name in layout_names {
        let Some(layout_name) = layout_name.as_deref() else {
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
                let path = pkg.theme_path(slot.key());
                slot.insert(pkg.budgeted(&path, |_| xml::theme::parse(&xml_text)));
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
            map.extend(pkg.budgeted(&part, |_| xml::comments::parse_authors(&x)));
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

/// 一份批注拷贝(不含回复)的模型字节:结构体 + 正文 + 时间(作者是共享的,不另计)。
fn comment_copy_bytes(c: &Comment) -> usize {
    std::mem::size_of::<Comment>()
        + c.text.as_ref().map_or(0, String::len)
        + c.datetime.as_ref().map_or(0, String::len)
}

/// 从 `src` 里按条数预算 `budget`(批注 + 回复合计)与模型字节预算拷贝前缀;返回
/// `(拷贝结果, 是否因字节预算提前停止)`。
fn take_comments(pkg: &Package, src: &[Comment], budget: &mut usize) -> (Vec<Comment>, bool) {
    let mut out = Vec::new();
    for c in src {
        if *budget == 0 {
            break;
        }
        if !pkg.take_model_bytes(comment_copy_bytes(c)) {
            return (out, true);
        }
        *budget -= 1;
        let mut copy = Comment {
            author: c.author.clone(),
            initials: c.initials.clone(),
            datetime: c.datetime.clone(),
            text: c.text.clone(),
            position: c.position,
            replies: Vec::new(),
        };
        for r in c.replies.iter().take(*budget) {
            if !pkg.take_model_bytes(comment_copy_bytes(r)) {
                out.push(copy);
                return (out, true);
            }
            *budget -= 1;
            copy.replies.push(r.clone());
        }
        out.push(copy);
    }
    (out, false)
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
        // 额度已用尽:不再解析(也不拷贝)该部件,只记截断——解析量按**剩余**额度封顶,
        // 而不是每个部件都按全局上限完整解析一遍。
        if cache.left == 0 {
            if pkg.has_part(&path) {
                pkg.note(DiagnosticKind::CommentsTruncated, &path, 1);
            }
            continue;
        }
        let max = cache.max.min(cache.left);
        let cached = cache
            .parsed
            .entry(path.clone())
            .or_insert_with(|| {
                pkg.part_str(&path).map(|x| {
                    let p =
                        pkg.budgeted(&path, |_| xml::comments::parse_comments(&x, authors, max));
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
        let (copies, out_of_bytes) = take_comments(pkg, comments, &mut cache.left);
        out.extend(copies);
        if *stopped || out_of_bytes || before < want {
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
        .map(|x| pkg.budgeted(&part, |_| xml::table_style::parse(&x)))
        .unwrap_or_default()
}

/// 节的 `sldId@id` 列表 → 幻灯片序号(经 `@id → 部件 → 序号`;解析不出的 id 丢弃)。
fn resolve_sections(
    sections: &[(String, Vec<u32>)],
    slide_ids: &[(u32, String)],
    part_index: &BTreeMap<String, usize>,
) -> Vec<Section> {
    let id_to_index: BTreeMap<u32, usize> = slide_ids
        .iter()
        .filter_map(|(id, target)| Some((*id, *part_index.get(target)?)))
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

/// 幻灯片部件列表:`presentation.xml` 的 `sldIdLst` 去重结果(解析时已按 `r:id` 折成部件路径、
/// 同一部件只保留首次出现);一个都没有时回退到按 `slideN` 数字升序(确定性兜底)。
/// 重复引用记 [`DiagnosticKind::DuplicateSlideRef`](`part` = 被重复引用的 slide 部件):合法文件里
/// 一个 slide 部件只会被引用一次,不去重则一个很小的文件就能把同一页放大成 N 份解析与存储。
/// 去重后的数量超过 [`ZipLimits::max_slides`] 返回 [`PptError::LimitExceeded`](解析在第
/// `max_slides + 1` 个不同部件处已停止收集,`actual` 因此是下界)。
fn resolve_slide_order(
    meta: &xml::presentation::PresentationMeta,
    pkg: &Package,
    limits: &ZipLimits,
) -> Result<Vec<String>> {
    for (target, n) in &meta.duplicate_refs {
        pkg.note(DiagnosticKind::DuplicateSlideRef, target, *n);
    }
    let mut parts = meta.slide_parts.clone();
    if parts.is_empty() {
        // 兜底:直接按 slide 文件名数字序。
        parts = pkg.slide_names_sorted();
    }
    if meta.too_many_slides || parts.len() > limits.max_slides {
        return Err(PptError::LimitExceeded {
            kind: LimitKind::Slides,
            limit: limits.max_slides as u64,
            actual: parts.len().max(limits.max_slides.saturating_add(1)) as u64,
        });
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    use super::*;
    use crate::xml::comments::PARSED_ENTRIES;

    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

    /// `slides` 张幻灯片,第 i 张经 rels 引用 `ppt/comments/comment{i}.xml`(内容 `cm(i)`)。
    fn comment_deck(slides: usize, cm: impl Fn(usize) -> String) -> Vec<u8> {
        let ns = r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
        let ids: String = (0..slides)
            .map(|i| format!(r#"<p:sldId id="{}" r:id="rId{i}"/>"#, 256 + i))
            .collect();
        let pres_rels: String = (0..slides)
            .map(|i| {
                format!(
                    r#"<Relationship Id="rId{i}" Type="{REL}/slide" Target="slides/slide{i}.xml"/>"#
                )
            })
            .collect();
        let wrap = |b: &str| {
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{b}</Relationships>"#
            )
        };
        let mut parts = vec![
            (
                "ppt/presentation.xml".to_string(),
                format!(r#"<p:presentation {ns}><p:sldIdLst>{ids}</p:sldIdLst></p:presentation>"#),
            ),
            (
                "ppt/_rels/presentation.xml.rels".to_string(),
                wrap(&pres_rels),
            ),
        ];
        for i in 0..slides {
            parts.push((
                format!("ppt/slides/slide{i}.xml"),
                format!(r#"<p:sld {ns}><p:cSld><p:spTree/></p:cSld></p:sld>"#),
            ));
            parts.push((
                format!("ppt/slides/_rels/slide{i}.xml.rels"),
                wrap(&format!(
                    r#"<Relationship Id="rId1" Type="{REL}/comments" Target="../comments/comment{i}.xml"/>"#
                )),
            ));
            parts.push((format!("ppt/comments/comment{i}.xml"), cm(i)));
        }
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buf);
            for (name, body) in &parts {
                zip.start_file(name.as_str(), SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    fn parse_counting(bytes: &[u8], max_comments: usize) -> (ParsedPptx, usize) {
        let limits = ZipLimits {
            max_comments,
            ..ZipLimits::default()
        };
        PARSED_ENTRIES.with(|c| c.set(0));
        let p = parse_bytes_with_limits(bytes, &limits).unwrap();
        (p, PARSED_ENTRIES.with(std::cell::Cell::get))
    }

    /// 40 张幻灯片各一个 1 000 条的批注部件、总预算 500:解析量按**剩余额度**封顶
    /// (不是每个部件都按全局上限完整解析一遍),额度用尽后的部件直接跳过不解析。
    #[test]
    fn comment_parsing_stops_at_the_remaining_budget() {
        let bytes = comment_deck(40, |_| {
            format!(
                r#"<p:cmLst xmlns:p="urn:p">{}</p:cmLst>"#,
                "<p:cm><p:text>x</p:text></p:cm>".repeat(1_000)
            )
        });
        let (p, parsed) = parse_counting(&bytes, 500);
        let kept: usize = p.presentation.slides.iter().map(|s| s.comments.len()).sum();
        assert_eq!(kept, 500);
        assert_eq!(parsed, 500, "只解析到剩余额度为止");
        let truncated = p
            .presentation
            .diagnostics
            .iter()
            .filter(|d| d.kind == DiagnosticKind::CommentsTruncated)
            .count();
        assert_eq!(truncated, 40, "每个被截断 / 跳过的部件都有一条诊断");
    }

    /// 线程式批注的回复同样计入解析额度。
    #[test]
    fn threaded_replies_count_against_the_parse_budget() {
        let bytes = comment_deck(10, |_| {
            format!(
                r#"<p188:cmLst xmlns:p188="urn:p188"><p188:cm><p188:replyLst>{}</p188:replyLst></p188:cm></p188:cmLst>"#,
                "<p188:reply/>".repeat(1_000)
            )
        });
        let (p, parsed) = parse_counting(&bytes, 300);
        let kept: usize = p
            .presentation
            .slides
            .iter()
            .flat_map(|s| &s.comments)
            .map(|c| 1 + c.replies.len())
            .sum();
        assert_eq!(kept, 300);
        assert_eq!(parsed, 300);
    }
}
