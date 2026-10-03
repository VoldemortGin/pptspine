//! 导出选项 + 每页"有序形状视图"(纯文本 / Markdown 共用):展平组合、配对继承链解析结果、
//! 按 [`TextOrder`] 排序;纯文本渲染也在这里。

use crate::geom::Emu;
use crate::model::{Chart, Presentation, Shape, Slide};
use crate::resolved::{ResolvedPresentation, ResolvedSlide};

use super::reading_order::{flatten, reading_order, FlatShape};
use super::{chart_label, chart_table, frame_text, notes_text, table_text};

/// 文字导出的形状顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextOrder {
    /// spTree 文档顺序(z 序;组合子形状就地展开)——历史行为,作回退。
    Document,
    /// 视觉阅读顺序(见 [`super::reading_order`];标题占位符总是最先)。
    #[default]
    Visual,
}

impl TextOrder {
    /// 从 `"document"` / `"visual"` 解析;其它取值 → `None`。
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "document" => Some(TextOrder::Document),
            "visual" => Some(TextOrder::Visual),
            _ => None,
        }
    }
}

/// 纯文本 / Markdown 导出选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExportOptions {
    /// 形状顺序(缺省视觉顺序)。
    pub order: TextOrder,
    /// 是否包含隐藏页(`p:sld@show="0"`;缺省跳过,与 PowerPoint 放映 / 导出一致)。
    pub include_hidden: bool,
}

/// 选项过滤后要导出的幻灯片(按演示顺序),各配上其解析结果(按 `index` 匹配)。
pub(crate) fn exported_slides<'a>(
    pres: &'a Presentation,
    resolved: Option<&'a ResolvedPresentation>,
    opts: &ExportOptions,
) -> impl Iterator<Item = (&'a Slide, Option<&'a ResolvedSlide>)> + 'a {
    let include_hidden = opts.include_hidden;
    pres.slides
        .iter()
        .filter(move |s| include_hidden || !s.hidden)
        .map(move |s| {
            let rs = resolved.and_then(|r| r.slides.iter().find(|rs| rs.index == s.index));
            (s, rs)
        })
}

/// 一张 slide 的叶子形状,按 `order` 排好。
pub(crate) fn ordered_shapes<'a>(
    slide: &'a Slide,
    resolved: Option<&'a ResolvedSlide>,
    slide_size: (Emu, Emu),
    order: TextOrder,
) -> Vec<FlatShape<'a>> {
    let flat = flatten(&slide.shapes, resolved.map(|r| r.shapes.as_slice()));
    if order == TextOrder::Document {
        return flat;
    }
    let rects: Vec<_> = flat.iter().map(|f| f.rect).collect();
    let mut ordered: Vec<FlatShape<'a>> = reading_order(&rects, slide_size)
        .into_iter()
        .map(|i| flat[i])
        .collect();
    // 标题占位符总是最先(稳定分区,其余相对顺序不变)。
    ordered.sort_by_key(|f| !is_title(f.shape));
    ordered
}

/// 形状的占位符种类(`@type` 缺省按 ECMA-376 记 `body`);非占位符为 `None`。
pub(crate) fn placeholder_kind(shape: &Shape) -> Option<&str> {
    let ph = match shape {
        Shape::TextBox(t) => t.placeholder.as_ref(),
        Shape::Auto(a) => a.placeholder.as_ref(),
        Shape::Picture(p) => p.placeholder.as_ref(),
        _ => None,
    }?;
    Some(ph.kind.as_deref().unwrap_or("body"))
}

/// 是否标题占位符(`title` / `ctrTitle`)。
pub(crate) fn is_title(shape: &Shape) -> bool {
    matches!(placeholder_kind(shape), Some("title" | "ctrTitle"))
}

/// 一张 slide 的正文文字(不含备注),按 `order` 排序。
pub fn slide_text_with(
    slide: &Slide,
    resolved: Option<&ResolvedSlide>,
    slide_size: (Emu, Emu),
    order: TextOrder,
) -> String {
    ordered_shapes(slide, resolved, slide_size, order)
        .iter()
        .filter_map(|f| shape_text(f.shape))
        .collect::<Vec<_>>()
        .join("\n")
}

fn shape_text(shape: &Shape) -> Option<String> {
    let s = match shape {
        Shape::TextBox(tf) => frame_text(tf),
        Shape::Auto(a) => a.text.as_deref().map(frame_text).unwrap_or_default(),
        Shape::Table(t) => table_text(t),
        Shape::Placeholder(p) => p.chart.as_ref().map(chart_text).unwrap_or_default(),
        Shape::Picture(_) | Shape::Connector(_) | Shape::Group(_) => String::new(),
    };
    (!s.is_empty()).then_some(s)
}

/// 图表纯文本:标题行(无标题为 `Chart (<kind>)`)+ 每类别一行 `类别: 值, 值`。
fn chart_text(c: &Chart) -> String {
    let mut lines = vec![c.title.clone().unwrap_or_else(|| chart_label(c))];
    if let Some((_, rows)) = chart_table(c) {
        lines.extend(
            rows.into_iter()
                .map(|r| format!("{}: {}", r[0], r[1..].join(", "))),
        );
    }
    lines.join("\n")
}

/// 整份演示文稿的纯文本:各 slide 以 `--- slide N ---` 分隔(N = 原 1 基序号,跳过隐藏页时
/// 序号不重排),附演讲者备注。`resolved` 提供占位符继承几何(视觉顺序更准);可为 `None`。
pub fn presentation_text_with(
    pres: &Presentation,
    resolved: Option<&ResolvedPresentation>,
    opts: &ExportOptions,
) -> String {
    let mut sections: Vec<String> = Vec::new();
    for (slide, rs) in exported_slides(pres, resolved, opts) {
        let mut sect = format!("--- slide {} ---", slide.index + 1);
        let body = slide_text_with(slide, rs, pres.slide_size, opts.order);
        if !body.is_empty() {
            sect.push('\n');
            sect.push_str(&body);
        }
        if let Some(notes) = notes_text(slide) {
            sect.push_str("\n\nNotes:\n");
            sect.push_str(&notes);
        }
        sections.push(sect);
    }
    sections.join("\n\n")
}
