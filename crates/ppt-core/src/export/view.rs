//! 导出选项 + 每页"有序形状视图"(纯文本 / Markdown 共用):展平组合、配对继承链解析结果、
//! 按 [`TextOrder`] 排序;纯文本渲染也在这里。

use crate::geom::Emu;
use crate::model::{Chart, Presentation, Shape, Slide};
use crate::resolved::{ResolvedPresentation, ResolvedSlide};

use super::markdown::{resolved_paragraphs, resolved_table};
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

/// 导出输出字节预算的缺省值。
///
/// 合法大文稿的实测输出:500 页 × 50 × 20 表格的纯文本约 6 MB、Markdown 约 10 MB;64 MiB
/// 留足 6 倍以上余量。共享的模型(同一图表被 N 个 frame 引用)在导出时仍按 frame 展开,
/// 这个上限让"小文件 × 大量 frame"的导出文本有界。
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

/// 纯文本 / Markdown 导出选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportOptions {
    /// 形状顺序(缺省视觉顺序)。
    pub order: TextOrder,
    /// 是否包含隐藏页(`p:sld@show="0"`;缺省跳过,与 PowerPoint 放映 / 导出一致)。
    pub include_hidden: bool,
    /// 输出字节上限(缺省 [`DEFAULT_MAX_OUTPUT_BYTES`]):达到后停止生成,按字符边界截断并在
    /// 末尾追加一行 [`TRUNCATION_MARKER`] 开头的标记;[`TextExport::truncated`] 同时置位。
    pub max_output_bytes: usize,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            order: TextOrder::default(),
            include_hidden: false,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

/// 截断标记行的前缀(完整标记形如 `[pptspine: output truncated at N bytes]`)。
pub const TRUNCATION_MARKER: &str = "[pptspine: output truncated";

/// 有界导出的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextExport {
    /// 导出文字(截断时末尾带标记行)。
    pub text: String,
    /// 是否因 [`ExportOptions::max_output_bytes`] 被截断。
    pub truncated: bool,
}

/// 有上限的输出缓冲:写满即停(按字符边界截断),之后的写入一律丢弃。
pub(crate) struct OutBuf {
    pub(crate) s: String,
    cap: usize,
    pub(crate) truncated: bool,
}

impl OutBuf {
    pub(crate) fn new(cap: usize) -> Self {
        OutBuf {
            s: String::new(),
            cap,
            truncated: false,
        }
    }

    /// 已写满(调用方据此停止生成后续内容)。
    pub(crate) fn full(&self) -> bool {
        self.truncated
    }

    pub(crate) fn push(&mut self, t: &str) {
        if self.truncated {
            return;
        }
        let room = self.cap - self.s.len();
        if t.len() <= room {
            self.s.push_str(t);
            return;
        }
        let mut end = room;
        while !t.is_char_boundary(end) {
            end -= 1;
        }
        self.s.push_str(&t[..end]);
        self.truncated = true;
    }

    pub(crate) fn finish(mut self) -> TextExport {
        if self.truncated {
            self.s
                .push_str(&format!("\n{TRUNCATION_MARKER} at {} bytes]", self.cap));
        }
        TextExport {
            text: self.s,
            truncated: self.truncated,
        }
    }
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
    let mut out = OutBuf::new(usize::MAX);
    slide_text_into(slide, resolved, slide_size, order, &mut out);
    out.s
}

/// 把一张 slide 的正文逐个形状写进 `out`(写满即停,不先拼出整页)。
fn slide_text_into(
    slide: &Slide,
    resolved: Option<&ResolvedSlide>,
    slide_size: (Emu, Emu),
    order: TextOrder,
    out: &mut OutBuf,
) {
    let mut first = true;
    for f in ordered_shapes(slide, resolved, slide_size, order) {
        if out.full() {
            return;
        }
        if let Some(t) = shape_text(&f) {
            if !first {
                out.push("\n");
            }
            first = false;
            out.push(&t);
        }
    }
}

fn shape_text(f: &FlatShape) -> Option<String> {
    let s = match f.shape {
        Shape::TextBox(tf) => frame_text(tf, resolved_paragraphs(f, tf.paragraphs.len())),
        Shape::Auto(a) => a
            .text
            .as_deref()
            .map(|tf| frame_text(tf, resolved_paragraphs(f, tf.paragraphs.len())))
            .unwrap_or_default(),
        Shape::Table(t) => table_text(t, resolved_table(f)),
        Shape::Placeholder(p) => match &p.chart {
            Some(c) => chart_text(c),
            None => p.diagram_text.join("\n"),
        },
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
    presentation_text_bounded(pres, resolved, opts).text
}

/// 同 [`presentation_text_with`],另报告是否因 [`ExportOptions::max_output_bytes`] 被截断。
pub fn presentation_text_bounded(
    pres: &Presentation,
    resolved: Option<&ResolvedPresentation>,
    opts: &ExportOptions,
) -> TextExport {
    let mut out = OutBuf::new(opts.max_output_bytes);
    for (n, (slide, rs)) in exported_slides(pres, resolved, opts).enumerate() {
        if out.full() {
            break;
        }
        if n > 0 {
            out.push("\n\n");
        }
        out.push(&format!("--- slide {} ---", slide.index + 1));
        let mut body = OutBuf::new(opts.max_output_bytes);
        slide_text_into(slide, rs, pres.slide_size, opts.order, &mut body);
        if !body.s.is_empty() {
            out.push("\n");
            out.push(&body.s);
        }
        if let Some(notes) = notes_text(slide) {
            out.push("\n\nNotes:\n");
            out.push(&notes);
        }
    }
    out.finish()
}
