#![forbid(unsafe_code)]
//! `ppt-render` —— 把继承链解析后的终态 IR([`ppt_core::resolved`])映射到共享
//! 排版引擎 `pdf-typeset`(pdfspine Phase A),产出**逐 slide 一页、绝对定位**的
//! 忠实 PDF(PRD-PDF-EXPORT §5,B-1/B-2)。
//!
//! 职责边界:本 crate 只做 IR → 引擎输入的**薄映射**;所有字体解析 / 布局 / PDF
//! 序列化都在 `pdf-typeset` 内。坐标契约:一切 op 以**左上原点、y 向下**的页坐标
//! 表达,发射时由引擎统一翻转(`pdf_typeset::ops` 的约定)。
//!
//! 本批覆盖:空白页装配 + 显式几何文本框(B-1/B-2)+ 预设形状底 fill/stroke +
//! xfrm rot/flip 与 avLst 调整值 + prstDash 虚线 + 图片放置(srcRect/fillRect)
//! (B-4)+ 组合仿射(B-5)+ 图表/SmartArt 占位框 + bodyPr 锚定/内边距/换行/
//! normAutofit 字号缩放(B-6)+ 表格网格/填充/文字/逐边框线(含 tcPr 边框/内边距/
//! 锚定,B-7)+ 幻灯片背景含 layout/master 继承(B-10,继承在 `ppt-parse::resolve`
//! 终态化)。

mod chart;
mod custgeom;
mod shapes;
mod text;
mod transform;

use std::collections::BTreeMap;
use std::path::Path;

use ppt_core::geom::emu_to_points;
use ppt_core::resolved::{ResolvedFill, ResolvedPresentation, ResolvedShape, ResolvedSlide};
use ppt_core::{PptError, Result};

pub use pdf_typeset::{ExportResult, ExportWarning};
use pdf_typeset::{Matrix, Op, PageOps, Rect, Typesetter};

use transform::{group_transform, Flatten, GroupTransform};

/// 渲染选项(PRD §5:`font_map` 覆盖喂给 TS-2 字体解析器)。
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    /// 请求字体族 → 覆盖目标。值为**存在的文件路径**时,把该字体文件注入解析器
    /// (文件须包含所请求的字体族);否则视为**替代字体族名**,叠加进 TS-2 替换表。
    pub font_map: BTreeMap<String, String>,
}

/// 把终态 IR 渲染成 PDF:每张 slide 一页,页面尺寸 = 画布尺寸(EMU → pt)。
///
/// 形状按 spTree 文档顺序绘制(即 OOXML 的 z-order);任何降级都进
/// [`ExportResult`] 的 `warnings`,**绝不 panic**。
///
/// # Errors
///
/// 引擎序列化失败折成 [`PptError::Render`]。
pub fn render_pdf(
    pres: &ResolvedPresentation,
    media: &BTreeMap<String, Vec<u8>>,
    opts: &RenderOptions,
) -> Result<ExportResult> {
    let mut ts = Typesetter::with_system_fonts();
    apply_font_map(&mut ts, opts);

    let (cx, cy) = pres.slide_size;
    // 画布缺失/畸形时兜底为 4:3 缺省(与解析层的容错哲学一致)。
    let (width, height) = if cx > 0 && cy > 0 {
        (emu_to_points(cx), emu_to_points(cy))
    } else {
        (720.0, 540.0)
    };

    let mut ctx = RenderCtx {
        media,
        image_ids: BTreeMap::new(),
        warnings: Vec::new(),
        vertical_warned: false,
        small_caps_warned: false,
        math_warned: false,
        accents: ppt_core::resolved::DEFAULT_ACCENTS,
    };
    let pages: Vec<PageOps> = pres
        .slides
        .iter()
        .map(|slide| PageOps {
            width,
            height,
            ops: slide_ops(&mut ts, &mut ctx, slide, width, height),
        })
        .collect();

    let mut result = ts
        .emit(&pages)
        .map_err(|e| PptError::Render(e.to_string()))?;
    result.warnings.extend(ctx.warnings);
    Ok(result)
}

/// 渲染期跨形状共享的状态:media 字节表、图片 id 缓存(同图多次放置只 embed 一份,
/// 键为 media 裸名 / rel id)、渲染侧告警。
struct RenderCtx<'a> {
    media: &'a BTreeMap<String, Vec<u8>>,
    image_ids: BTreeMap<String, Option<usize>>,
    warnings: Vec<ExportWarning>,
    /// 纵排文字降级告警的一次性开关(整篇 presentation 只发一条)。
    vertical_warned: bool,
    /// 小型大写(`cap=small`)降级告警的一次性开关(整篇只发一条)。
    small_caps_warned: bool,
    /// 公式 run 按普通文本降级告警的一次性开关(整篇只发一条)。
    math_warned: bool,
    /// 当前 slide 的主题 accent1..6(图表系列配色)。
    accents: [[u8; 3]; 6],
}

impl RenderCtx<'_> {
    /// 公式 run(`RunKind::Math`,文本是线性化纯文本)无排版,按普通文字画;整篇首次遇到时告警一次。
    fn note_math(&mut self, paragraphs: &[ppt_core::resolved::ResolvedParagraph]) {
        if self.math_warned {
            return;
        }
        let has_math = paragraphs
            .iter()
            .flat_map(|p| &p.runs)
            .any(|r| r.kind == ppt_core::model::RunKind::Math);
        if has_math {
            self.warnings.push(ExportWarning::Custom {
                kind: "math-text".into(),
                detail: "公式(a14:m)v1 按线性化纯文本绘制,不做数学排版".into(),
            });
            self.math_warned = true;
        }
    }

    /// `cap=small` 引擎无小型大写,`text::run_input` 近似为全大写;整篇首次遇到时告警一次。
    fn note_small_caps(&mut self, paragraphs: &[ppt_core::resolved::ResolvedParagraph]) {
        if self.small_caps_warned {
            return;
        }
        let small = paragraphs
            .iter()
            .flat_map(|p| &p.runs)
            .any(|r| r.cap == ppt_core::style::Caps::Small);
        if small {
            self.warnings.push(ExportWarning::Custom {
                kind: "small-caps".into(),
                detail: "小型大写(rPr@cap=small)v1 近似为全大写".into(),
            });
            self.small_caps_warned = true;
        }
    }
}

/// 一张 slide 的全部绘制 op(背景先铺,再 master → layout 继承形状,最后 slide
/// spTree;各层内文档顺序 = 绘制顺序)。
fn slide_ops(
    ts: &mut Typesetter,
    ctx: &mut RenderCtx<'_>,
    slide: &ResolvedSlide,
    width: f64,
    height: f64,
) -> Vec<Op> {
    let mut ops = Vec::new();
    ctx.accents = slide.accents;
    // B-10:整页背景(`ResolvedSlide.background` 已含 slide → layout → master 继承)。
    if let Some(bg) = &slide.background {
        background_ops(ts, ctx, bg, width, height, &mut ops);
    }
    for shape in slide.inherited_shapes.iter().chain(&slide.shapes) {
        shape_ops(ts, ctx, shape, Flatten::IDENTITY, &mut ops);
    }
    ops
}

/// B-10:整页背景 → 满页填充 / 图片。
fn background_ops(
    ts: &mut Typesetter,
    ctx: &mut RenderCtx<'_>,
    bg: &ppt_core::resolved::ResolvedBackground,
    width: f64,
    height: f64,
    ops: &mut Vec<Op>,
) {
    use ppt_core::resolved::ResolvedBackground;
    match bg {
        ResolvedBackground::Color(fill) => {
            if matches!(fill, ResolvedFill::Gradient(_)) {
                ctx.warnings.push(ExportWarning::GradientDegraded {
                    kind: "bgFill".to_string(),
                });
            }
            if matches!(fill, ResolvedFill::Pattern(_)) {
                shapes::pattern_fill_warning(ctx);
            }
            let c = text::rgb(fill.color());
            ops.push(Op::FillRect {
                x: 0.0,
                y: 0.0,
                w: width,
                h: height,
                color: c,
            });
        }
        ResolvedBackground::Picture { media_name } => {
            let key = format!("bg::{media_name}");
            let id = if let Some(&cached) = ctx.image_ids.get(&key) {
                cached
            } else {
                let id = match ctx.media.get(media_name) {
                    Some(bytes) => {
                        ts.add_image(&pdf_typeset::ImageSpec::new(bytes.clone(), width, height))
                    }
                    None => {
                        ctx.warnings.push(ExportWarning::ImageDropped {
                            reason: format!("background media '{media_name}' not found"),
                        });
                        None
                    }
                };
                ctx.image_ids.insert(key, id);
                id
            };
            if let Some(id) = id {
                ops.push(Op::Image {
                    id,
                    x: 0.0,
                    y: 0.0,
                    w: width,
                    h: height,
                });
            }
        }
    }
}

/// 文本体 → op(矩形经组合仿射映射;pptx `rot` 顺时针 → 引擎视觉逆时针取负;
/// 翻转不镜像文字,PowerPoint 语义)。
fn text_ops(
    ts: &mut Typesetter,
    ctx: &mut RenderCtx<'_>,
    rect: Option<ppt_core::geom::Rect>,
    tf: &ppt_core::resolved::ResolvedTextFrame,
    flat: Flatten,
    ops: &mut Vec<Op>,
) {
    let Some(rect) = rect else {
        return;
    };
    // Task 4:纵排文字(bodyPr@vert)v1 仍按水平排版降级(不改 text_box_spec),
    // 但一次性发出降级告警(整篇只一条)。
    if tf.body.vertical && !ctx.vertical_warned {
        ctx.warnings.push(ExportWarning::Custom {
            kind: "vertical-text".into(),
            detail: "纵排文字(bodyPr@vert)v1 水平降级".into(),
        });
        ctx.vertical_warned = true;
    }
    ctx.note_small_caps(&tf.paragraphs);
    ctx.note_math(&tf.paragraphs);
    let mapped = flat.map_emu_rect(rect);
    let rot = -tf.xfrm.rot_deg();
    // B-6:`normAutofit` 生效但**未存** fontScale → 用引擎 TS-10 测量按内容重算缩放;
    // 已存 fontScale(stored)走既有 `text_box_spec` 透传路径,行为不回归。
    let spec = if tf.body.autofit_normal && tf.body.font_scale.is_none() {
        text::autofit_text_box_spec(ts, mapped, rot, &tf.body, &tf.paragraphs)
    } else {
        text::text_box_spec(mapped, rot, &tf.body, &tf.paragraphs)
    };
    ops.extend(ts.layout_text_box(&spec));
}

/// 单个形状 → op 序列。`flat` 是**已累积**的组合仿射(子坐标 → 页坐标,B-5):
/// 纯平移 + 均匀正缩放的组合预乘进坐标(文本框与"拆组等价形状"逐字重合);
/// 含旋转/翻转/非均匀缩放的组合退回 `Op::Group { transform }`(引擎 `q cm … Q`
/// 嵌套自然复合),其内子形状回到恒等仿射。
fn shape_ops(
    ts: &mut Typesetter,
    ctx: &mut RenderCtx<'_>,
    shape: &ResolvedShape,
    flat: Flatten,
    ops: &mut Vec<Op>,
) {
    match shape {
        ResolvedShape::TextBox(tf) => text_ops(ts, ctx, tf.rect, tf, flat, ops),
        ResolvedShape::Auto(auto) => {
            shapes::auto_shape_ops(ts, ctx, auto, flat, ops);
            if let Some(tf) = &auto.text {
                text_ops(ts, ctx, tf.rect.or(auto.rect), tf, flat, ops);
            }
        }
        ResolvedShape::Connector(conn) => shapes::connector_ops(ctx, conn, flat, ops),
        ResolvedShape::Picture(pic) => shapes::picture_ops(ts, ctx, pic, flat, ops),
        ResolvedShape::Group(g) => match group_transform(g) {
            GroupTransform::Flat(f) => {
                let combined = flat.after(f);
                for child in &g.children {
                    shape_ops(ts, ctx, child, combined, ops);
                }
            }
            GroupTransform::Full(m) => {
                let mut inner = Vec::new();
                for child in &g.children {
                    shape_ops(ts, ctx, child, Flatten::IDENTITY, &mut inner);
                }
                if !inner.is_empty() {
                    ops.push(Op::Group {
                        transform: Some(Matrix::concat(&m, &flat.to_matrix())),
                        clip: None,
                        ops: inner,
                    });
                }
            }
        },
        ResolvedShape::Placeholder(gp) => shapes::graphic_placeholder_ops(ts, ctx, gp, flat, ops),
        // B-7:绝对单元格网格 + 填充 + 文字 + 逐边框线。
        ResolvedShape::Table(t) => table_ops(ts, ctx, t, flat, ops),
    }
}

/// B-7:表格 → 单元格网格。列宽按 `col_widths` 比例铺满表格矩形;行高按声明值
/// 比例铺满(全缺省时等分);单元格填充 / 文字(用 `tcPr` 内边距 + 锚定)/ 逐边框线;
/// `gridSpan` 横跨、`rowSpan` 纵跨(合并延续格跳过绘制)。表格样式已在解析层合进单元格
/// 终态;`tableStyleId` 未解析(找不到 / 部件缺失)时记一次降级。
fn table_ops(
    ts: &mut Typesetter,
    ctx: &mut RenderCtx<'_>,
    table: &ppt_core::resolved::ResolvedTable,
    flat: Flatten,
    ops: &mut Vec<Op>,
) {
    let Some(rect) = table.rect else {
        return;
    };
    if table.table_style_id.is_some() && !table.style_resolved {
        ctx.warnings.push(ExportWarning::Custom {
            kind: "table-style".to_string(),
            detail: "tableStyleId 在 tableStyles.xml 中找不到;仅绘制直接填充 / 边框".to_string(),
        });
    }
    let r = flat.map_emu_rect(rect);
    // `a:tblGrid` 缺失(非法但需容错)→ 按最大行格数把表宽等分,绝不整表丢弃
    // (charter:degrade-never-drop)。列数取各行格数最大值(每个 `a:tc` 恰占一列)。
    let ncols = if table.col_widths.is_empty() {
        let n = table.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
        if n > 0 {
            ctx.warnings.push(ExportWarning::Custom {
                kind: "table-grid".to_string(),
                detail: "tblGrid 列宽缺失;按最大行格数等分表宽降级".to_string(),
            });
        }
        n
    } else {
        table.col_widths.len()
    };
    if ncols == 0 || table.rows.is_empty() {
        return;
    }
    // 列 x 边界:按 col_widths 比例铺满表宽(无声明列宽时等分)。
    let total_w: f64 = table.col_widths.iter().map(|w| emu_to_points(*w)).sum();
    let table_w = r.x1 - r.x0;
    let mut xs = Vec::with_capacity(ncols + 1);
    let mut acc = r.x0;
    xs.push(acc);
    for i in 0..ncols {
        let frac = match table.col_widths.get(i) {
            Some(w) if total_w > 0.0 => emu_to_points(*w) / total_w,
            _ => 1.0 / ncols as f64,
        };
        acc += frac * table_w;
        xs.push(acc);
    }
    // 行 y 边界:B-7 按内容自适应增高——每行取 `max(声明行高, 内容所需高)`,内容高
    // 经引擎 TS-10 测量(与真实排版逐点一致);表格顶端锚定,内容超声明则向下增长。
    let nrows = table.rows.len();
    let heights = row_heights(ts, table, &xs, ncols, r);
    let mut ys = Vec::with_capacity(nrows + 1);
    let mut yacc = r.y0;
    ys.push(yacc);
    for h in &heights {
        yacc += h;
        ys.push(yacc);
    }

    for (ri, row) in table.rows.iter().enumerate() {
        // 每个 `a:tc` 恰占一个网格列(gridSpan 首格宽绘 + 后随 hMerge 占位格)。
        for (c, cell) in row.cells.iter().enumerate() {
            if c >= ncols {
                break;
            }
            if !cell.merged {
                let cspan = (cell.col_span.max(1) as usize).min(ncols - c);
                let rspan = (cell.row_span.max(1) as usize).min(nrows - ri);
                let crect = Rect::new(xs[c], ys[ri], xs[c + cspan], ys[ri + rspan]);
                if let Some(fill) = cell.fill {
                    ops.push(Op::FillRect {
                        x: crect.x0,
                        y: crect.y0,
                        w: crect.x1 - crect.x0,
                        h: crect.y1 - crect.y0,
                        color: text::rgb(fill),
                    });
                }
                cell_border_ops(&cell.borders, crect, ops);
                let body = cell_body(cell);
                ctx.note_small_caps(&cell.paragraphs);
                ctx.note_math(&cell.paragraphs);
                let spec = text::text_box_spec(crect, 0.0, &body, &cell.paragraphs);
                ops.extend(ts.layout_text_box(&spec));
            }
        }
    }
}

/// B-7:按内容自适应行高——每行取 `max(声明行高, 内容所需高)`。内容高经引擎 TS-10
/// 测量([`Typesetter::measure_text_box`],与真实排版逐点一致):单元格内容按列宽换行
/// 度量,加 `tcPr` 上下内边距即该单元格所需外高。`rowSpan` 单元格的内容高在其纵跨的
/// 各行间**均摊**补足(单行单元格先定基,再看跨行格是否需要在跨行总高上补差)。
fn row_heights(
    ts: &mut Typesetter,
    table: &ppt_core::resolved::ResolvedTable,
    xs: &[f64],
    ncols: usize,
    r: Rect,
) -> Vec<f64> {
    let nrows = table.rows.len();
    let table_h = r.y1 - r.y0;
    let declared: Vec<Option<f64>> = table
        .rows
        .iter()
        .map(|row| row.height.map(emu_to_points))
        .collect();
    // 基线行高(B-7 之前的既有几何):声明全在 → 按比例铺满声明表高;否则等分。
    // 自适应只在内容**超过**基线时增高,内容放得下则与旧输出逐点一致(不无谓漂移)。
    let all_known = declared.iter().all(Option::is_some);
    let sum_known: f64 = declared.iter().flatten().sum();
    let base = |i: usize| -> f64 {
        if all_known && sum_known > 0.0 {
            declared[i].unwrap() / sum_known * table_h
        } else if nrows > 0 {
            table_h / nrows as f64
        } else {
            0.0
        }
    };
    let mut needed = vec![0.0_f64; nrows];
    // (起始行, 纵跨行数, 所需外高)——rowSpan 单元格留待第二遍在跨行间均摊。
    let mut spans: Vec<(usize, usize, f64)> = Vec::new();
    for (ri, row) in table.rows.iter().enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            if c >= ncols {
                break;
            }
            if cell.merged {
                continue;
            }
            let cspan = (cell.col_span.max(1) as usize).min(ncols - c);
            let rspan = (cell.row_span.max(1) as usize).min(nrows - ri);
            let cell_w = xs[c + cspan] - xs[c];
            // 任意占位高度(measure 只取宽度换行);内容高 = 度量高 + tcPr 上下内边距。
            let body = cell_body(cell);
            let spec = text::text_box_spec(
                Rect::new(xs[c], r.y0, xs[c] + cell_w, r.y0 + 1.0e6),
                0.0,
                &body,
                &cell.paragraphs,
            );
            let outer = ts.measure_text_box(&spec).height + emu_to_points(cell.mar_t + cell.mar_b);
            if rspan <= 1 {
                needed[ri] = needed[ri].max(outer);
            } else {
                spans.push((ri, rspan, outer));
            }
        }
    }
    let mut heights: Vec<f64> = (0..nrows).map(|i| base(i).max(needed[i])).collect();
    // rowSpan:跨行格所需外高超过其纵跨各行之和时,把差额均摊到这些行。
    for (ri, rspan, outer) in spans {
        let sum: f64 = heights[ri..ri + rspan].iter().sum();
        if outer > sum {
            let add = (outer - sum) / rspan as f64;
            for h in &mut heights[ri..ri + rspan] {
                *h += add;
            }
        }
    }
    heights
}

/// 单元格逐边框线:边框已由解析层(`tcPr`/`tcBorders`)填充并终态化进
/// `ResolvedCellBorders`,这里逐边(上/下/左/右)绘制。
fn cell_border_ops(b: &ppt_core::resolved::ResolvedCellBorders, r: Rect, ops: &mut Vec<Op>) {
    let mut edge = |s: &Option<ppt_core::resolved::ResolvedStroke>, x1, y1, x2, y2| {
        if let Some(st) = s {
            let color = st
                .color
                .map(text::rgb)
                .unwrap_or_else(|| pdf_typeset::Rgb::new(0.0, 0.0, 0.0));
            let width = st.width_emu.map(emu_to_points).unwrap_or(0.75);
            ops.push(Op::Line {
                x1,
                y1,
                x2,
                y2,
                color,
                width,
            });
        }
    };
    edge(&b.top, r.x0, r.y0, r.x1, r.y0);
    edge(&b.bottom, r.x0, r.y1, r.x1, r.y1);
    edge(&b.left, r.x0, r.y0, r.x0, r.y1);
    edge(&b.right, r.x1, r.y0, r.x1, r.y1);
}

/// 单元格 `tcPr` 内边距 + 锚定 → 复用文本框 bodyPr 通道。
fn cell_body(cell: &ppt_core::resolved::ResolvedCell) -> ppt_core::resolved::ResolvedBodyProps {
    ppt_core::resolved::ResolvedBodyProps {
        anchor: cell.anchor,
        l_ins: cell.mar_l,
        r_ins: cell.mar_r,
        t_ins: cell.mar_t,
        b_ins: cell.mar_b,
        ..Default::default()
    }
}

/// 把 `font_map` 覆盖应用到引擎的字体解析器(必须在任何布局之前)。
fn apply_font_map(ts: &mut Typesetter, opts: &RenderOptions) {
    for (requested, target) in &opts.font_map {
        let path = Path::new(target);
        if path.is_file() {
            if let Ok(bytes) = std::fs::read(path) {
                ts.resolver_mut().add_font_data(bytes);
            }
        } else {
            ts.resolver_mut()
                .add_substitution(requested, &[target.as_str()]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppt_core::color::ResolvedColor;
    use ppt_core::geom::Rect;
    use ppt_core::model::{RunKind, Xfrm};
    use ppt_core::resolved::{
        ResolvedAutoShape, ResolvedBullet, ResolvedConnector, ResolvedFill, ResolvedGroup,
        ResolvedParagraph, ResolvedRun, ResolvedStroke, ResolvedTextFrame,
    };

    /// 2×2 RGB PNG(左上红 / 右上绿 / 左下蓝 / 右下黄),srcRect 裁剪测试用。
    const TINY_PNG: [u8; 77] = [
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 2,
        0, 0, 0, 253, 212, 154, 115, 0, 0, 0, 20, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 192,
        0, 194, 12, 255, 255, 255, 103, 0, 0, 30, 239, 4, 252, 163, 200, 180, 247, 0, 0, 0, 0, 73,
        69, 78, 68, 174, 66, 96, 130,
    ];

    fn run(text: &str) -> ResolvedRun {
        ResolvedRun {
            text: text.into(),
            kind: RunKind::Text,
            font: Some("Arial".into()),
            ea_font: None,
            cs_font: None,
            size_pt: 20.0,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color: ResolvedColor::opaque([0, 0, 0]),
            char_spacing_pt: 0.0,
            baseline: 0.0,
            cap: ppt_core::style::Caps::None,
            link: None,
        }
    }

    fn para(text: &str) -> ResolvedParagraph {
        ResolvedParagraph {
            level: 0,
            align: None,
            mar_l: None,
            indent: None,
            ln_spc: None,
            spc_bef: None,
            spc_aft: None,
            bullet: ResolvedBullet::None,
            runs: vec![run(text)],
        }
    }

    fn text_box(rect: Rect, rot: i32, text: &str) -> ResolvedShape {
        ResolvedShape::TextBox(ResolvedTextFrame {
            rect: Some(rect),
            xfrm: Xfrm {
                rot,
                flip_h: false,
                flip_v: false,
            },
            body: ppt_core::resolved::ResolvedBodyProps::default(),
            paragraphs: vec![para(text)],
        })
    }

    fn pres(slides: Vec<ResolvedSlide>) -> ResolvedPresentation {
        ResolvedPresentation {
            slide_size: (12_192_000, 6_858_000), // 16:9 => 960x540 pt
            slides,
        }
    }

    fn one_slide(shapes: Vec<ResolvedShape>) -> ResolvedPresentation {
        pres(vec![ResolvedSlide {
            index: 0,
            background: None,
            inherited_shapes: vec![],
            shapes,
            accents: ppt_core::resolved::DEFAULT_ACCENTS,
        }])
    }

    fn render(p: &ResolvedPresentation) -> ExportResult {
        render_pdf(p, &BTreeMap::new(), &RenderOptions::default()).expect("render")
    }

    fn connector(rect: Rect, xfrm: Xfrm, dash: Option<&str>) -> ResolvedShape {
        ResolvedShape::Connector(ResolvedConnector {
            rect: Some(rect),
            xfrm,
            geometry: Some("straightConnector1".into()),
            adjusts: vec![],
            fill: None,
            stroke: Some(ResolvedStroke {
                color: Some(ResolvedColor::opaque([255, 0, 0])),
                width_emu: Some(12_700), // 1 pt
                dash: dash.map(str::to_string),
                head_end: None,
                tail_end: None,
            }),
            no_line: false,
            custom_geometry: false,
            cust_geom: None,
        })
    }

    fn auto_shape(rect: Rect, geometry: &str, adjusts: Vec<(String, i64)>) -> ResolvedShape {
        ResolvedShape::Auto(ResolvedAutoShape {
            rect: Some(rect),
            xfrm: Xfrm::default(),
            geometry: Some(geometry.into()),
            adjusts,
            fill: Some(ResolvedFill::Solid(ResolvedColor::opaque([255, 0, 0]))),
            blip_fill: None,
            stroke: None,
            text: None,
            custom_geometry: false,
            cust_geom: None,
        })
    }

    #[test]
    fn b10_background_fills_full_page() {
        use ppt_core::resolved::ResolvedBackground;
        let slide = ResolvedSlide {
            index: 0,
            background: Some(ResolvedBackground::Color(ResolvedFill::Solid(
                ResolvedColor::opaque([255, 0, 0]),
            ))),
            inherited_shapes: vec![],
            shapes: vec![],
            accents: ppt_core::resolved::DEFAULT_ACCENTS,
        };
        let out = render(&pres(vec![slide]));
        let hay = String::from_utf8_lossy(&out.pdf);
        // 满页红色背景填充(内容流不压缩,可 grep)。
        assert!(hay.contains("1 0 0 rg"), "background red fill missing");
    }

    /// 图案填充背景:画两色平均色满页 + 与形状同一条 `pattern-fill-degraded` 告警。
    #[test]
    fn pattern_background_is_flat_color_with_the_shared_warning() {
        use ppt_core::resolved::ResolvedBackground;
        let slide = ResolvedSlide {
            index: 0,
            background: Some(ResolvedBackground::Color(ResolvedFill::Pattern(
                ResolvedColor::opaque([255, 0, 0]),
            ))),
            inherited_shapes: vec![],
            shapes: vec![],
            accents: ppt_core::resolved::DEFAULT_ACCENTS,
        };
        let out = render(&pres(vec![slide]));
        assert_eq!(
            custom_kinds(&out),
            vec!["pattern-fill-degraded".to_string()]
        );
        assert!(String::from_utf8_lossy(&out.pdf).contains("1 0 0 rg"));
    }

    #[test]
    fn b7_table_renders_cell_fill_and_grid() {
        use ppt_core::resolved::{ResolvedCell, ResolvedCellBorders, ResolvedRow, ResolvedTable};
        let mk_cell = |t: &str, fill: Option<ResolvedColor>| ResolvedCell {
            paragraphs: vec![para(t)],
            col_span: 1,
            row_span: 1,
            fill,
            merged: false,
            mar_l: 91_440,
            mar_r: 91_440,
            mar_t: 45_720,
            mar_b: 45_720,
            anchor: ppt_core::resolved::ResolvedAnchor::Top,
            borders: ResolvedCellBorders::default(),
        };
        let table = ResolvedShape::Table(ResolvedTable {
            rect: Some(Rect::new(0, 0, 6_000_000, 2_000_000)),
            col_widths: vec![3_000_000, 3_000_000],
            table_style_id: None,
            style_resolved: false,
            rows: vec![ResolvedRow {
                cells: vec![
                    mk_cell("A1", Some(ResolvedColor::opaque([0, 0, 255]))),
                    mk_cell("B1", None),
                ],
                height: Some(2_000_000),
            }],
        });
        let out = render(&one_slide(vec![table]));
        let hay = String::from_utf8_lossy(&out.pdf);
        // 蓝色单元格填充落在内容流。
        assert!(hay.contains("0 0 1 rg"), "cell blue fill missing");
        // 单元格文字经引擎排版(BT/ET 文本块存在)。
        assert!(hay.contains("BT"), "table cell text missing");
    }

    /// `table-style` 降级告警只在 styleId 未解析(找不到 / 部件缺失)时发;已解析不告警。
    #[test]
    fn table_style_warning_only_when_unresolved() {
        use ppt_core::resolved::{ResolvedCell, ResolvedCellBorders, ResolvedRow, ResolvedTable};
        let table = |resolved: bool| {
            ResolvedShape::Table(ResolvedTable {
                rect: Some(Rect::new(0, 0, 6_000_000, 2_000_000)),
                col_widths: vec![6_000_000],
                table_style_id: Some("{X}".into()),
                style_resolved: resolved,
                rows: vec![ResolvedRow {
                    cells: vec![ResolvedCell {
                        paragraphs: vec![para("A1")],
                        col_span: 1,
                        row_span: 1,
                        fill: None,
                        merged: false,
                        mar_l: 91_440,
                        mar_r: 91_440,
                        mar_t: 45_720,
                        mar_b: 45_720,
                        anchor: ppt_core::resolved::ResolvedAnchor::Top,
                        borders: ResolvedCellBorders::default(),
                    }],
                    height: Some(2_000_000),
                }],
            })
        };
        let warned = |resolved: bool| {
            render(&one_slide(vec![table(resolved)]))
                .warnings
                .iter()
                .any(|w| matches!(w, ExportWarning::Custom { kind, .. } if kind == "table-style"))
        };
        assert!(warned(false), "未解析的 styleId 必须告警");
        assert!(!warned(true), "已解析的样式不告警");
    }

    #[test]
    fn b7_table_without_tbl_grid_degrades_to_even_columns() {
        use ppt_core::resolved::{ResolvedCell, ResolvedCellBorders, ResolvedRow, ResolvedTable};
        let mk_cell = |t: &str, fill: Option<ResolvedColor>| ResolvedCell {
            paragraphs: vec![para(t)],
            col_span: 1,
            row_span: 1,
            fill,
            merged: false,
            mar_l: 91_440,
            mar_r: 91_440,
            mar_t: 45_720,
            mar_b: 45_720,
            anchor: ppt_core::resolved::ResolvedAnchor::Top,
            borders: ResolvedCellBorders::default(),
        };
        // tblGrid 缺失(col_widths 空):按最大行格数等分,绝不整表丢弃 + 一条降级告警。
        let table = ResolvedShape::Table(ResolvedTable {
            rect: Some(Rect::new(0, 0, 6_000_000, 2_000_000)),
            col_widths: vec![],
            table_style_id: None,
            style_resolved: false,
            rows: vec![ResolvedRow {
                cells: vec![
                    mk_cell("A1", Some(ResolvedColor::opaque([0, 0, 255]))),
                    mk_cell("B1", None),
                ],
                height: Some(2_000_000),
            }],
        });
        let out = render(&one_slide(vec![table]));
        let hay = String::from_utf8_lossy(&out.pdf);
        assert!(
            hay.contains("0 0 1 rg"),
            "cell fill must survive missing tblGrid"
        );
        assert!(hay.contains("BT"), "cell text must survive missing tblGrid");
        assert!(
            out.warnings
                .iter()
                .any(|w| matches!(w, ExportWarning::Custom { kind, .. } if kind == "table-grid")),
            "even-split degradation must warn once"
        );
    }

    #[test]
    fn b7_cell_borders_draw_stroked_lines() {
        use ppt_core::resolved::{
            ResolvedCell, ResolvedCellBorders, ResolvedRow, ResolvedStroke, ResolvedTable,
        };
        // 红色左边框(width 12700 EMU = 1 pt)。
        let red_left = ResolvedCellBorders {
            left: Some(ResolvedStroke {
                color: Some(ResolvedColor::opaque([255, 0, 0])),
                width_emu: Some(12_700),
                dash: None,
                head_end: None,
                tail_end: None,
            }),
            ..ResolvedCellBorders::default()
        };
        let cell = ResolvedCell {
            paragraphs: vec![para("A1")],
            col_span: 1,
            row_span: 1,
            fill: None,
            merged: false,
            mar_l: 91_440,
            mar_r: 91_440,
            mar_t: 45_720,
            mar_b: 45_720,
            anchor: ppt_core::resolved::ResolvedAnchor::Top,
            borders: red_left,
        };
        let table = ResolvedShape::Table(ResolvedTable {
            rect: Some(Rect::new(0, 0, 4_000_000, 1_500_000)),
            col_widths: vec![4_000_000],
            table_style_id: None,
            style_resolved: false,
            rows: vec![ResolvedRow {
                cells: vec![cell],
                height: Some(1_500_000),
            }],
        });
        let out = render(&one_slide(vec![table]));
        let hay = String::from_utf8_lossy(&out.pdf);
        // 红色描边 + 描线路径(moveto/lineto/stroke)落在内容流。
        assert!(hay.contains("1 0 0 RG"), "cell border stroke color missing");
        assert!(hay.contains(" l S"), "cell border line path missing");
    }

    /// B-7 按内容自适应行高:列窄、字大的长文单元格,行高**超过**声明值(内容撑高)。
    #[test]
    fn b7_row_grows_to_fit_long_cell_content() {
        use ppt_core::resolved::{ResolvedCell, ResolvedCellBorders, ResolvedRow, ResolvedTable};
        let long = "word ".repeat(80);
        let cell = ResolvedCell {
            paragraphs: vec![para(&long)],
            col_span: 1,
            row_span: 1,
            fill: None,
            merged: false,
            mar_l: 0,
            mar_r: 0,
            mar_t: 0,
            mar_b: 0,
            anchor: ppt_core::resolved::ResolvedAnchor::Top,
            borders: ResolvedCellBorders::default(),
        };
        let table = ResolvedTable {
            rect: Some(Rect::new(0, 0, 2_540_000, 127_000)), // 200 × 10 pt
            col_widths: vec![2_540_000],
            table_style_id: None,
            style_resolved: false,
            rows: vec![ResolvedRow {
                cells: vec![cell],
                height: Some(127_000), // 声明仅 10pt
            }],
        };
        let mut ts = Typesetter::with_system_fonts();
        let xs = [0.0, 200.0];
        let r = pdf_typeset::Rect::new(0.0, 0.0, 200.0, 10.0);
        let heights = row_heights(&mut ts, &table, &xs, 1, r);
        assert!(
            heights[0] > 10.0,
            "行高应随内容增高、超过声明的 10pt,实测 {}",
            heights[0]
        );
        // 200pt 宽、20pt 字的长文必然折多行 → 行高远超单行。
        assert!(heights[0] > 60.0, "长文应撑出多行行高,实测 {}", heights[0]);
    }

    /// B-7 rowSpan 高度分摊:纵跨 2 行的长内容格,两行无声明高 → 内容高**均摊**到两行。
    #[test]
    fn b7_rowspan_distributes_content_height_across_rows() {
        use ppt_core::resolved::{
            ResolvedAnchor, ResolvedCell, ResolvedCellBorders, ResolvedRow, ResolvedTable,
        };
        let mk = |paras: Vec<ResolvedParagraph>, rspan: u32, merged: bool| ResolvedCell {
            paragraphs: paras,
            col_span: 1,
            row_span: rspan,
            fill: None,
            merged,
            mar_l: 0,
            mar_r: 0,
            mar_t: 0,
            mar_b: 0,
            anchor: ResolvedAnchor::Top,
            borders: ResolvedCellBorders::default(),
        };
        let long = "word ".repeat(40);
        let table = ResolvedTable {
            rect: Some(Rect::new(0, 0, 2_540_000, 254_000)),
            col_widths: vec![2_540_000],
            table_style_id: None,
            style_resolved: false,
            rows: vec![
                // 行 0:rowSpan=2 的长内容格;行 1:合并延续格(跳过)。两行均无声明高。
                ResolvedRow {
                    cells: vec![mk(vec![para(&long)], 2, false)],
                    height: None,
                },
                ResolvedRow {
                    cells: vec![mk(vec![], 1, true)],
                    height: None,
                },
            ],
        };
        let mut ts = Typesetter::with_system_fonts();
        let xs = [0.0, 200.0];
        let r = pdf_typeset::Rect::new(0.0, 0.0, 200.0, 20.0);
        let heights = row_heights(&mut ts, &table, &xs, 1, r);
        assert_eq!(heights.len(), 2);
        assert!(
            heights[0] > 0.0 && heights[1] > 0.0,
            "跨行内容高应均摊到两行,实测 {heights:?}"
        );
        assert!(
            (heights[0] - heights[1]).abs() < 1e-6,
            "两行应等额均摊,实测 {heights:?}"
        );
        assert!(
            heights[0] + heights[1] > 40.0,
            "跨行合计应容纳多行内容,实测 {heights:?}"
        );
    }

    #[test]
    fn b6_bottom_anchor_places_text_lower() {
        // 底部锚定的文本框:验证渲染不 panic + 产出有效非空 PDF(锚定几何由引擎
        // TS-5 保证,此处只钉住 bodyPr 接线通路)。
        let body = ppt_core::resolved::ResolvedBodyProps {
            anchor: ppt_core::resolved::ResolvedAnchor::Bottom,
            ..Default::default()
        };
        let tf = ResolvedTextFrame {
            rect: Some(Rect::new(0, 0, 4_000_000, 3_000_000)),
            xfrm: Xfrm::default(),
            body,
            paragraphs: vec![para("anchored")],
        };
        let out = render(&one_slide(vec![ResolvedShape::TextBox(tf)]));
        assert!(out.pdf.starts_with(b"%PDF-"));
        assert!(String::from_utf8_lossy(&out.pdf).contains("BT"));
    }

    /// B-6 重算式 autofit:超框长文启用 `normAutofit`(未存 fontScale)后**完整落入
    /// 框内**且**字号缩小**——用引擎 TS-10 测量验收(与真实排版逐点一致)。
    #[test]
    fn b6_recompute_autofit_shrinks_long_text_into_box() {
        // 小框(200×60 pt);normAutofit 生效但未存 fontScale → 走重算路径。
        let body = ppt_core::resolved::ResolvedBodyProps {
            autofit_normal: true,
            ..Default::default()
        };
        // 6 段短文(20pt,单倍行距 ~24pt/行 → 100% 需 ~144pt,远超框高 → 必溢出;
        // 缩到 ~55% 可落入,恰在 25% 下限之上)。
        let paras: Vec<ResolvedParagraph> = (0..6).map(|i| para(&format!("Line {i}"))).collect();
        let box_rect = pdf_typeset::Rect::new(0.0, 0.0, 200.0, 90.0);

        let mut ts = Typesetter::with_system_fonts();
        // 基线(100%,text_box_spec)在该小框内确实溢出。
        let base = text::text_box_spec(box_rect, 0.0, &body, &paras);
        let inner_h = (base.rect.y1 - base.rect.y0).abs();
        assert!(
            ts.measure_text_box(&base).height > inner_h,
            "长文在 100% 应溢出小框"
        );

        // 重算 autofit 后:内容高度落入框内。
        let fitted = text::autofit_text_box_spec(&mut ts, box_rect, 0.0, &body, &paras);
        assert!(
            ts.measure_text_box(&fitted).height <= inner_h + 1.0,
            "autofit 后内容应完整落入框内(inner_h={inner_h})"
        );
        // 且字号确实缩小(< 原始 20pt)。
        let size = match &fitted.blocks[0] {
            pdf_typeset::Block::Paragraph(_, runs) => runs[0].style.size,
            _ => panic!("首块应为段落"),
        };
        assert!(size < 20.0, "autofit 应缩小字号,实测 {size}pt");
    }

    /// B-6:normAutofit 生效但内容**本就**放得下时,不缩放(字号保持原值)。
    #[test]
    fn b6_autofit_noop_when_content_fits() {
        let body = ppt_core::resolved::ResolvedBodyProps {
            autofit_normal: true,
            ..Default::default()
        };
        let paras = vec![para("short")];
        let box_rect = pdf_typeset::Rect::new(0.0, 0.0, 400.0, 300.0);
        let mut ts = Typesetter::with_system_fonts();
        let fitted = text::autofit_text_box_spec(&mut ts, box_rect, 0.0, &body, &paras);
        let size = match &fitted.blocks[0] {
            pdf_typeset::Block::Paragraph(_, runs) => runs[0].style.size,
            _ => panic!("首块应为段落"),
        };
        assert_eq!(size, 20.0, "放得下时不应缩放字号");
    }

    #[test]
    fn blank_slides_export_one_page_each() {
        let p = pres(vec![
            ResolvedSlide {
                index: 0,
                background: None,
                inherited_shapes: vec![],
                shapes: vec![],
                accents: ppt_core::resolved::DEFAULT_ACCENTS,
            },
            ResolvedSlide {
                index: 1,
                background: None,
                inherited_shapes: vec![],
                shapes: vec![],
                accents: ppt_core::resolved::DEFAULT_ACCENTS,
            },
        ]);
        let out = render(&p);
        assert!(out.pdf.starts_with(b"%PDF-"));
        // 两页:页面树 /Count 2。
        let hay = String::from_utf8_lossy(&out.pdf);
        assert!(hay.contains("/Count 2"), "expected 2-page tree");
    }

    #[test]
    fn text_box_embeds_exactly_one_face() {
        let out = render(&one_slide(vec![text_box(
            Rect::new(914_400, 914_400, 4_572_000, 1_828_800),
            0,
            "Hello render",
        )]));
        let count = out
            .pdf
            .windows(b"/FontFile2".len())
            .filter(|w| w == b"/FontFile2")
            .count();
        assert_eq!(count, 1, "exactly one FontFile2 per used face");
    }

    /// 带链接的文本框:`link` 为 `Some` 的整段 run(`plain` 为不带链接的前后缀)。
    fn linked_box(rect: Rect, link: Option<&str>, text: &str) -> ResolvedShape {
        let mut linked = run(text);
        linked.link = link.map(str::to_string);
        let ResolvedShape::TextBox(mut tf) = text_box(rect, 0, "") else {
            unreachable!()
        };
        tf.paragraphs[0].runs = vec![run("see "), linked, run(" end")];
        ResolvedShape::TextBox(tf)
    }

    fn link_annots(pdf: &[u8]) -> usize {
        String::from_utf8_lossy(pdf)
            .matches("/Subtype /Link")
            .count()
    }

    /// 外链 run → 页面 `/Link` 注释(URI 动作);只放行 http / https / mailto;自动换行的
    /// 链接每行一个矩形;没有链接 / 被拒 scheme 则无注释。
    #[test]
    fn run_hyperlinks_become_uri_link_annotations() {
        let wide = Rect::new(914_400, 914_400, 9_144_000, 1_828_800);
        let narrow = Rect::new(914_400, 914_400, 2_286_000, 3_657_600);
        let long = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";

        let out = render(&one_slide(vec![linked_box(
            wide,
            Some("https://example.com/a?b=c"),
            "click me",
        )]));
        let hay = String::from_utf8_lossy(&out.pdf);
        assert_eq!(link_annots(&out.pdf), 1);
        assert!(hay.contains("/URI (https://example.com/a?b=c)"), "URI 动作");

        let mail = render(&one_slide(vec![linked_box(
            wide,
            Some("mailto:a@b.co"),
            "mail",
        )]));
        assert_eq!(link_annots(&mail.pdf), 1);

        for bad in [
            Some("javascript:alert(1)"),
            Some("file:///etc/passwd"),
            Some("ppaction://hlinksldjump"),
            None,
        ] {
            let out = render(&one_slide(vec![linked_box(wide, bad, "click me")]));
            assert_eq!(link_annots(&out.pdf), 0, "{bad:?}");
            assert!(
                !String::from_utf8_lossy(&out.pdf).contains("/URI"),
                "{bad:?}"
            );
        }

        let wrapped = render(&one_slide(vec![linked_box(
            narrow,
            Some("https://example.com"),
            long,
        )]));
        assert!(link_annots(&wrapped.pdf) >= 2, "跨行链接每行一个矩形");
        // 字节确定。
        let again = render(&one_slide(vec![linked_box(
            narrow,
            Some("https://example.com"),
            long,
        )]));
        assert_eq!(wrapped.pdf, again.pdf);
    }

    #[test]
    fn unknown_preset_degrades_with_warning() {
        let out = render(&one_slide(vec![auto_shape(
            Rect::new(0, 0, 914_400, 914_400),
            "cloud",
            vec![],
        )]));
        assert!(out
            .warnings
            .iter()
            .any(|w| matches!(w, ExportWarning::PresetDegraded { preset } if preset == "cloud")));
    }

    fn arrow_connector(geometry: &str, tail: ppt_core::model::LineEndKind) -> ResolvedShape {
        let ResolvedShape::Connector(mut c) =
            connector(Rect::new(0, 0, 2_540_000, 0), Xfrm::default(), None)
        else {
            unreachable!()
        };
        c.geometry = Some(geometry.into());
        if let Some(s) = c.stroke.as_mut() {
            s.tail_end = Some(ppt_core::model::LineEnd {
                kind: tail,
                ..ppt_core::model::LineEnd::default()
            });
        }
        ResolvedShape::Connector(c)
    }

    fn custom_kinds(out: &ExportResult) -> Vec<String> {
        out.warnings
            .iter()
            .filter_map(|w| match w {
                ExportWarning::Custom { kind, .. } => Some(kind.clone()),
                _ => None,
            })
            .collect()
    }

    /// 尾端三角:多出一个线色实心填充(`f`),线身被缩短,无告警。
    #[test]
    fn connector_tail_triangle_draws_filled_head() {
        use ppt_core::model::LineEndKind;
        let plain = render(&one_slide(vec![arrow_connector(
            "straightConnector1",
            LineEndKind::None,
        )]));
        let arrow = render(&one_slide(vec![arrow_connector(
            "straightConnector1",
            LineEndKind::Triangle,
        )]));
        assert!(arrow.warnings.is_empty(), "{:?}", arrow.warnings);
        let hay = String::from_utf8_lossy(&arrow.pdf);
        assert!(
            hay.contains("1 0 0 rg"),
            "arrow fill must use the line color"
        );
        // 200 pt 长的线:线身止于 200 − 3×max(1pt, 0.7mm) ≈ 194.05。
        assert!(
            hay.contains("194.0"),
            "line body must stop at the arrow base"
        );
        // `type="none"` 与无线端字节一致。
        let bare = render(&one_slide(vec![connector(
            Rect::new(0, 0, 2_540_000, 0),
            Xfrm::default(),
            None,
        )]));
        assert_eq!(plain.pdf, bare.pdf);
    }

    #[test]
    fn unknown_line_end_kind_warns_and_skips() {
        use ppt_core::model::LineEndKind;
        let out = render(&one_slide(vec![arrow_connector(
            "straightConnector1",
            LineEndKind::Other("bogus".into()),
        )]));
        assert_eq!(custom_kinds(&out), vec!["line-end-degraded".to_string()]);
    }

    /// 子集外连接线预设降级成闭合包围盒:箭头无处可放 → 降级告警。
    #[test]
    fn degraded_connector_preset_drops_line_ends_with_warning() {
        use ppt_core::model::LineEndKind;
        let out = render(&one_slide(vec![arrow_connector(
            "curvedConnector3",
            LineEndKind::Triangle,
        )]));
        assert!(out
            .warnings
            .iter()
            .any(|w| matches!(w, ExportWarning::PresetDegraded { .. })));
        assert_eq!(custom_kinds(&out), vec!["line-end-degraded".to_string()]);
    }

    /// custGeom:有填充 → 包围盒矩形 + 告警;无填充无描边 → 不画、不告警。
    #[test]
    fn custom_geometry_approximated_with_warning() {
        let ResolvedShape::Auto(mut filled) =
            auto_shape(Rect::new(0, 0, 914_400, 914_400), "rect", vec![])
        else {
            unreachable!()
        };
        filled.geometry = None;
        filled.custom_geometry = true;
        let mut bare = filled.clone();
        bare.fill = None;

        let out = render(&one_slide(vec![ResolvedShape::Auto(filled)]));
        assert_eq!(
            custom_kinds(&out),
            vec!["custom-geometry-approximated".to_string()]
        );
        assert!(String::from_utf8_lossy(&out.pdf).contains("1 0 0 rg"));

        let out = render(&one_slide(vec![ResolvedShape::Auto(bare)]));
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    /// 构造一个带 `custGeom` 的红色填充自选图形(100pt x 100pt)。
    fn cust_shape(cust: Option<ppt_core::custgeom::CustGeom>, stroke: bool) -> ResolvedShape {
        let ResolvedShape::Auto(mut a) =
            auto_shape(Rect::new(0, 0, 1_270_000, 1_270_000), "rect", vec![])
        else {
            unreachable!()
        };
        a.geometry = None;
        a.custom_geometry = true;
        a.cust_geom = cust.map(Box::new);
        if stroke {
            a.stroke = Some(ResolvedStroke {
                color: Some(ResolvedColor::opaque([0, 0, 255])),
                width_emu: Some(12_700),
                dash: None,
                head_end: None,
                tail_end: None,
            });
        }
        ResolvedShape::Auto(a)
    }

    fn cust_path(cmds: Vec<ppt_core::custgeom::PathCmd>) -> ppt_core::custgeom::CustPath {
        ppt_core::custgeom::CustPath {
            w: Some(100),
            h: Some(100),
            fill: ppt_core::custgeom::PathFill::Norm,
            stroke: true,
            extrusion_ok: false,
            cmds,
        }
    }

    fn cpt(x: &str, y: &str) -> ppt_core::custgeom::CustPt {
        ppt_core::custgeom::CustPt {
            x: x.into(),
            y: y.into(),
        }
    }

    /// 内容流里 `l`(lineto)与 `c`(curveto)运算符的个数。
    fn count_ops(pdf: &[u8], op: &str) -> usize {
        String::from_utf8_lossy(pdf)
            .lines()
            .filter(|l| l.trim_end().ends_with(&format!(" {op}")))
            .count()
    }

    /// 求值成功的 custGeom:画真实三角形(2 条 `l`,而非包围盒的 3 条),且不发近似告警。
    #[test]
    fn custom_geometry_triangle_is_drawn_without_warning() {
        use ppt_core::custgeom::{CustGeom, PathCmd};
        let tri = CustGeom {
            paths: vec![cust_path(vec![
                PathCmd::MoveTo(cpt("0", "100")),
                PathCmd::LnTo(cpt("100", "100")),
                PathCmd::LnTo(cpt("50", "0")),
                PathCmd::Close,
            ])],
            ..CustGeom::default()
        };
        let out = render(&one_slide(vec![cust_shape(Some(tri), false)]));
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let pdf = String::from_utf8_lossy(&out.pdf);
        assert!(pdf.contains("1 0 0 rg"));
        assert_eq!(count_ops(&out.pdf, "l"), 2, "triangle, not a 3-line rect");
    }

    /// 带 arcTo 的路径输出曲线;两次渲染字节一致。
    #[test]
    fn custom_geometry_arc_draws_curves_deterministically() {
        use ppt_core::custgeom::{CustGeom, PathCmd};
        let g = CustGeom {
            paths: vec![cust_path(vec![
                PathCmd::MoveTo(cpt("100", "50")),
                PathCmd::ArcTo {
                    w_r: "50".into(),
                    h_r: "50".into(),
                    st_ang: "0".into(),
                    sw_ang: "21600000".into(),
                },
                PathCmd::Close,
            ])],
            ..CustGeom::default()
        };
        let shapes = || vec![cust_shape(Some(g.clone()), true)];
        let a = render(&one_slide(shapes()));
        let b = render(&one_slide(shapes()));
        assert!(a.warnings.is_empty(), "{:?}", a.warnings);
        assert_eq!(count_ops(&a.pdf, "c"), 4);
        assert_eq!(a.pdf, b.pdf, "byte-identical across renders");
    }

    /// 多个 path 各按自己的 fill / stroke 画:`fill="none"` 的 path 只描边(无 `f`),
    /// 另一条只填充(`stroke="0"`)。
    #[test]
    fn custom_geometry_paths_follow_their_own_fill_and_stroke() {
        use ppt_core::custgeom::{CustGeom, PathCmd, PathFill};
        let mut open = cust_path(vec![
            PathCmd::MoveTo(cpt("0", "0")),
            PathCmd::LnTo(cpt("100", "100")),
        ]);
        open.fill = PathFill::None;
        let mut solid = cust_path(vec![
            PathCmd::MoveTo(cpt("0", "0")),
            PathCmd::LnTo(cpt("100", "0")),
            PathCmd::LnTo(cpt("100", "100")),
            PathCmd::Close,
        ]);
        solid.stroke = false;
        let g = CustGeom {
            paths: vec![open, solid],
            ..CustGeom::default()
        };
        let out = render(&one_slide(vec![cust_shape(Some(g), true)]));
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let pdf = String::from_utf8_lossy(&out.pdf);
        let painters: Vec<&str> = pdf
            .lines()
            .map(str::trim)
            .filter(|l| matches!(*l, "S" | "s" | "f" | "f*" | "B" | "B*" | "b" | "b*"))
            .collect();
        assert_eq!(painters.len(), 2, "{painters:?}");
        assert!(
            painters[0].eq_ignore_ascii_case("s"),
            "stroke only: {painters:?}"
        );
        assert!(painters[1].starts_with('f'), "fill only: {painters:?}");
    }

    /// 求值失败(未定义名字)→ 退回包围盒 + 原近似告警,不 panic。
    #[test]
    fn custom_geometry_eval_failure_falls_back_with_warning() {
        use ppt_core::custgeom::{CustGeom, PathCmd};
        let bad = CustGeom {
            paths: vec![cust_path(vec![PathCmd::MoveTo(cpt("nosuch", "0"))])],
            ..CustGeom::default()
        };
        let out = render(&one_slide(vec![cust_shape(Some(bad), false)]));
        assert_eq!(
            custom_kinds(&out),
            vec!["custom-geometry-approximated".to_string()]
        );
        assert!(String::from_utf8_lossy(&out.pdf).contains("1 0 0 rg"));
    }

    #[test]
    fn missing_media_reports_image_dropped() {
        let out = render(&one_slide(vec![ResolvedShape::Picture(
            ppt_core::model::Picture {
                rect: Some(Rect::new(0, 0, 914_400, 914_400)),
                xfrm: Xfrm::default(),
                rel_id: "rId1".into(),
                media_name: Some("missing.png".into()),
                image_bytes_len: 0,
                src_rect: None,
                fill_rect: None,
                placeholder: None,
                ..Default::default()
            },
        )]));
        assert!(out
            .warnings
            .iter()
            .any(|w| matches!(w, ExportWarning::ImageDropped { .. })));
    }

    // ---- B-4:形状变换 / 虚线 / avLst / srcRect ---------------------------------

    /// prstDash="dash"、线宽 1 pt → 内容流出现 `[4 3] 0 d`(DrawingML 4/3 线宽单位)。
    #[test]
    fn dashed_stroke_emits_dash_pattern() {
        let rect = Rect::new(914_400, 914_400, 1_828_800, 914_400);
        let out = render(&one_slide(vec![connector(
            rect,
            Xfrm::default(),
            Some("dash"),
        )]));
        let hay = String::from_utf8_lossy(&out.pdf);
        assert!(hay.contains("[4 3] 0 d"), "expected dash pattern op");

        let solid = render(&one_slide(vec![connector(rect, Xfrm::default(), None)]));
        let solid_hay = String::from_utf8_lossy(&solid.pdf);
        assert!(
            !solid_hay.contains(" d\n"),
            "solid stroke must not set dashes"
        );
    }

    /// 显式无线(`no_line`)的连接线不画描边;无描边解析结果且非显式无线仍走缺省黑线兜底。
    #[test]
    fn connector_no_line_draws_nothing_but_default_still_black() {
        let rect = Rect::new(914_400, 914_400, 1_828_800, 914_400);
        let with = |no_line: bool, keep_stroke: bool| {
            let ResolvedShape::Connector(mut c) = connector(rect, Xfrm::default(), None) else {
                unreachable!()
            };
            c.no_line = no_line;
            if !keep_stroke {
                c.stroke = None;
            }
            let out = render(&one_slide(vec![ResolvedShape::Connector(c)]));
            String::from_utf8_lossy(&out.pdf).into_owned()
        };
        let strokes = |hay: &str| hay.lines().any(|l| l.trim() == "S" || l.ends_with(" RG"));
        assert!(!strokes(&with(true, false)), "显式无线不画线");
        assert!(
            strokes(&with(false, false)),
            "无描边且非显式无线 → 缺省黑线"
        );
    }

    /// flipH 连接线与不翻转的连接线产出不同内容(方向经形状级变换生效)。
    #[test]
    fn flipped_connector_differs_from_plain() {
        let rect = Rect::new(914_400, 914_400, 1_828_800, 914_400);
        let plain = render(&one_slide(vec![connector(rect, Xfrm::default(), None)]));
        let flipped = render(&one_slide(vec![connector(
            rect,
            Xfrm {
                rot: 0,
                flip_h: true,
                flip_v: false,
            },
            None,
        )]));
        assert!(String::from_utf8_lossy(&flipped.pdf).contains(" cm"));
        assert_ne!(plain.pdf, flipped.pdf, "flip must change emitted geometry");
    }

    /// 旋转 45° 的文本框与不旋转的产出不同内容(经引擎 rotation_deg → `cm`)。
    #[test]
    fn rotated_text_box_differs_from_plain() {
        let rect = Rect::new(914_400, 914_400, 3_657_600, 914_400);
        let plain = render(&one_slide(vec![text_box(rect, 0, "Spin")]));
        let rotated = render(&one_slide(vec![text_box(rect, 45 * 60_000, "Spin")]));
        assert!(String::from_utf8_lossy(&rotated.pdf).contains(" cm"));
        assert_ne!(plain.pdf, rotated.pdf);
    }

    /// roundRect 的 avLst 调整值透传 TS-6:adj=50000 与缺省产出不同轮廓。
    #[test]
    fn round_rect_adjust_changes_outline() {
        let rect = Rect::new(914_400, 914_400, 2_743_200, 1_828_800);
        let by_default = render(&one_slide(vec![auto_shape(rect, "roundRect", vec![])]));
        let adjusted = render(&one_slide(vec![auto_shape(
            rect,
            "roundRect",
            vec![("adj".into(), 50_000)],
        )]));
        assert_ne!(
            by_default.pdf, adjusted.pdf,
            "avLst adj must reach the outline"
        );
        assert!(by_default.warnings.is_empty() && adjusted.warnings.is_empty());
    }

    /// 渐变填充降级为代表色 + `GradientDegraded` 告警。
    #[test]
    fn gradient_fill_degrades_with_warning() {
        let out = render(&one_slide(vec![ResolvedShape::Auto(ResolvedAutoShape {
            rect: Some(Rect::new(0, 0, 914_400, 914_400)),
            xfrm: Xfrm::default(),
            geometry: Some("rect".into()),
            adjusts: vec![],
            fill: Some(ResolvedFill::Gradient(ResolvedColor::opaque([0, 0, 255]))),
            blip_fill: None,
            stroke: None,
            text: None,
            custom_geometry: false,
            cust_geom: None,
        })]));
        assert!(out
            .warnings
            .iter()
            .any(|w| matches!(w, ExportWarning::GradientDegraded { .. })));
    }

    /// srcRect 裁剪:整图放大铺放 + 显示矩形剪裁(`W n`),与不裁剪产出不同。
    #[test]
    fn src_rect_crops_via_enlarged_clipped_placement() {
        let media: BTreeMap<String, Vec<u8>> = [("tiny.png".to_string(), TINY_PNG.to_vec())].into();
        let pic = |src_rect| {
            one_slide(vec![ResolvedShape::Picture(ppt_core::model::Picture {
                rect: Some(Rect::new(914_400, 914_400, 1_828_800, 1_828_800)),
                xfrm: Xfrm::default(),
                rel_id: "rId1".into(),
                media_name: Some("tiny.png".into()),
                image_bytes_len: TINY_PNG.len(),
                src_rect,
                fill_rect: None,
                placeholder: None,
                ..Default::default()
            })])
        };
        let plain = render_pdf(&pic(None), &media, &RenderOptions::default()).expect("render");
        // 裁掉右半(r=50000):整图宽放大 2 倍、负偏移铺放、按显示矩形剪裁。
        let cropped = render_pdf(
            &pic(Some(ppt_core::model::RelRect {
                l: 0,
                t: 0,
                r: 50_000,
                b: 0,
            })),
            &media,
            &RenderOptions::default(),
        )
        .expect("render");
        let hay = String::from_utf8_lossy(&cropped.pdf);
        assert!(hay.contains("W n"), "expected clip path for srcRect crop");
        assert!(
            hay.contains("288 0 0 144"),
            "expected width doubled (144->288 pt)"
        );
        assert!(!String::from_utf8_lossy(&plain.pdf).contains("W n"));
        assert!(plain.warnings.is_empty() && cropped.warnings.is_empty());
    }

    // ---- 形状图片 / 图案填充 ---------------------------------------------------

    fn blip(src_rect: Option<ppt_core::model::RelRect>, tile: bool) -> ppt_core::model::BlipFill {
        ppt_core::model::BlipFill {
            rel_id: "rId1".into(),
            media_name: Some("tiny.png".into()),
            src_rect,
            fill_rect: None,
            tile,
        }
    }

    fn blip_shape(geometry: &str, b: ppt_core::model::BlipFill) -> ResolvedShape {
        ResolvedShape::Auto(ResolvedAutoShape {
            rect: Some(Rect::new(914_400, 914_400, 1_828_800, 1_828_800)),
            xfrm: Xfrm::default(),
            geometry: Some(geometry.into()),
            adjusts: vec![],
            fill: None,
            blip_fill: Some(b),
            stroke: None,
            text: None,
            custom_geometry: false,
            cust_geom: None,
        })
    }

    fn tiny_media() -> BTreeMap<String, Vec<u8>> {
        [("tiny.png".to_string(), TINY_PNG.to_vec())].into()
    }

    fn render_media(p: &ResolvedPresentation) -> ExportResult {
        render_pdf(p, &tiny_media(), &RenderOptions::default()).expect("render")
    }

    fn image_objects(out: &ExportResult) -> usize {
        String::from_utf8_lossy(&out.pdf)
            .matches("/Subtype /Image")
            .count()
    }

    /// 矩形的图片填充:画出图片并按几何裁剪(`W n`),无降级告警。
    #[test]
    fn blip_fill_rect_draws_image_clipped_to_geometry() {
        let out = render_media(&one_slide(vec![blip_shape("rect", blip(None, false))]));
        let hay = String::from_utf8_lossy(&out.pdf);
        assert_eq!(image_objects(&out), 1);
        assert!(hay.contains("W n"), "image must be clipped to the shape");
        assert!(hay.contains(" Do"), "image must be painted");
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    /// 椭圆的图片填充:裁剪路径是椭圆(含贝塞尔曲线),不是外接矩形;无告警。
    #[test]
    fn blip_fill_ellipse_clips_with_curves_not_the_bounding_box() {
        let ell = render_media(&one_slide(vec![blip_shape("ellipse", blip(None, false))]));
        let rect = render_media(&one_slide(vec![blip_shape("rect", blip(None, false))]));
        let (e, r) = (
            String::from_utf8_lossy(&ell.pdf).into_owned(),
            String::from_utf8_lossy(&rect.pdf).into_owned(),
        );
        assert!(
            e.contains("W n") && e.contains(" c\n"),
            "ellipse clip needs curves"
        );
        assert!(!r.contains(" c\n"));
        assert!(ell.warnings.is_empty(), "{:?}", ell.warnings);
    }

    /// 带 `srcRect`:整图放大 + 嵌套剪裁(裁剪外再套形状剪裁),宽度翻倍。
    #[test]
    fn blip_fill_src_rect_enlarges_the_placement() {
        let sr = ppt_core::model::RelRect {
            l: 0,
            t: 0,
            r: 50_000,
            b: 0,
        };
        let out = render_media(&one_slide(vec![blip_shape("rect", blip(Some(sr), false))]));
        let hay = String::from_utf8_lossy(&out.pdf);
        assert!(hay.contains("288 0 0 144"), "width must double");
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    /// 同一图片被形状填充与 `p:pic` 同时引用:只嵌入一次。
    #[test]
    fn blip_fill_and_picture_share_one_embedded_image() {
        let pic = ResolvedShape::Picture(ppt_core::model::Picture {
            rect: Some(Rect::new(0, 0, 914_400, 914_400)),
            rel_id: "rId1".into(),
            media_name: Some("tiny.png".into()),
            image_bytes_len: TINY_PNG.len(),
            ..Default::default()
        });
        let both = render_media(&one_slide(vec![
            pic,
            blip_shape("ellipse", blip(None, false)),
        ]));
        assert_eq!(image_objects(&both), 1);
        let hay = String::from_utf8_lossy(&both.pdf);
        assert_eq!(
            hay.matches(" Do").count(),
            2,
            "painted twice, embedded once"
        );
    }

    /// 平铺按拉伸画并告警;图片缺失 → `ImageDropped`,形状不崩。
    #[test]
    fn blip_fill_tile_and_missing_media_degrade_with_warnings() {
        let tiled = render_media(&one_slide(vec![blip_shape("rect", blip(None, true))]));
        assert_eq!(image_objects(&tiled), 1);
        assert!(
            has_custom_warning(&tiled, "blip-fill-tiled"),
            "{:?}",
            tiled.warnings
        );
        let missing = render(&one_slide(vec![blip_shape("rect", blip(None, false))]));
        assert!(missing
            .warnings
            .iter()
            .any(|w| matches!(w, ExportWarning::ImageDropped { .. })));
    }

    /// 图案填充按平均色纯色画 + `pattern-fill-degraded` 告警(不是无填充)。
    #[test]
    fn pattern_fill_paints_flat_color_with_warning() {
        let out = render(&one_slide(vec![ResolvedShape::Auto(ResolvedAutoShape {
            rect: Some(Rect::new(0, 0, 914_400, 914_400)),
            xfrm: Xfrm::default(),
            geometry: Some("rect".into()),
            adjusts: vec![],
            fill: Some(ResolvedFill::Pattern(ResolvedColor::opaque([0, 0, 255]))),
            blip_fill: None,
            stroke: None,
            text: None,
            custom_geometry: false,
            cust_geom: None,
        })]));
        assert!(String::from_utf8_lossy(&out.pdf).contains("0 0 1 rg"));
        assert!(
            has_custom_warning(&out, "pattern-fill-degraded"),
            "{:?}",
            out.warnings
        );
    }

    // ---- B-5:组合仿射 ----------------------------------------------------------

    /// 平移 + 均匀缩放的组合与"拆组等价形状"**逐字节**一致(孪生等价门)。
    #[test]
    fn flat_group_matches_preflattened_twin() {
        // 组合:child (0,0,144,72)pt → rect (72,72,288,144)pt,s=2。
        let grouped = one_slide(vec![ResolvedShape::Group(ResolvedGroup {
            rect: Some(Rect::new(914_400, 914_400, 3_657_600, 1_828_800)),
            child_rect: Some(Rect::new(0, 0, 1_828_800, 914_400)),
            xfrm: Xfrm::default(),
            children: vec![text_box(Rect::new(0, 0, 1_828_800, 914_400), 0, "Twin")],
        })]);
        let twin = one_slide(vec![text_box(
            Rect::new(914_400, 914_400, 3_657_600, 1_828_800),
            0,
            "Twin",
        )]);
        assert_eq!(render(&grouped).pdf, render(&twin).pdf);
    }

    /// 两层嵌套组合(各自平移 + 缩放)与手工推导的等价形状逐字节一致。
    #[test]
    fn nested_flat_groups_match_preflattened_twin() {
        // 内层:child (0,0,72,36)pt → (36,18,144,72)pt;外层:child (0,0,216,108)pt
        // → (72,72,432,216)pt。复合映射文本框 (0,0,72,36) → (144,108,288,144)pt。
        let inner = ResolvedShape::Group(ResolvedGroup {
            rect: Some(Rect::new(457_200, 228_600, 1_828_800, 914_400)),
            child_rect: Some(Rect::new(0, 0, 914_400, 457_200)),
            xfrm: Xfrm::default(),
            children: vec![text_box(Rect::new(0, 0, 914_400, 457_200), 0, "Deep")],
        });
        let outer = one_slide(vec![ResolvedShape::Group(ResolvedGroup {
            rect: Some(Rect::new(914_400, 914_400, 5_486_400, 2_743_200)),
            child_rect: Some(Rect::new(0, 0, 2_743_200, 1_371_600)),
            xfrm: Xfrm::default(),
            children: vec![inner],
        })]);
        let twin = one_slide(vec![text_box(
            Rect::new(1_828_800, 1_371_600, 3_657_600, 1_828_800),
            0,
            "Deep",
        )]);
        assert_eq!(render(&outer).pdf, render(&twin).pdf);
    }

    fn styled_text_box(text: &str, f: impl Fn(&mut ResolvedRun)) -> ResolvedShape {
        let mut p = para(text);
        f(&mut p.runs[0]);
        ResolvedShape::TextBox(ResolvedTextFrame {
            rect: Some(Rect::new(0, 0, 6_000_000, 3_000_000)),
            xfrm: Xfrm::default(),
            body: ppt_core::resolved::ResolvedBodyProps::default(),
            paragraphs: vec![p],
        })
    }

    fn has_custom_warning(out: &ExportResult, want: &str) -> bool {
        out.warnings
            .iter()
            .any(|w| matches!(w, ExportWarning::Custom { kind, .. } if kind == want))
    }

    fn has_spacing_fallback(out: &ExportResult) -> bool {
        out.warnings
            .iter()
            .any(|w| matches!(w, ExportWarning::SignedSpacingFallback { .. }))
    }

    /// 负字符间距:可前进的紧缩照常排、无回退告警;紧缩过度(簇不前进)时引擎只把
    /// 该段负间距归零 + `signed-spacing-fallback` 告警,文本照常渲染、不 panic。
    #[test]
    fn negative_char_spacing_condenses_or_falls_back_with_warning() {
        let mild = render(&one_slide(vec![styled_text_box("Condensed", |r| {
            r.char_spacing_pt = -1.0;
        })]));
        assert!(mild.pdf.starts_with(b"%PDF"));
        assert!(
            !has_spacing_fallback(&mild),
            "-1pt 可前进,不应回退: {:?}",
            mild.warnings
        );
        let extreme = render(&one_slide(vec![styled_text_box("Condensed", |r| {
            r.char_spacing_pt = -100.0;
        })]));
        assert!(extreme.pdf.starts_with(b"%PDF"));
        assert!(has_spacing_fallback(&extreme));
    }

    /// 上下标 / 间距真的改变输出;显式 0 与缺省逐字节一致(默认路径不变)。
    #[test]
    fn script_and_spacing_change_output_but_zero_is_identity() {
        let plain = render(&one_slide(vec![styled_text_box("x2", |_| {})])).pdf;
        let zero = render(&one_slide(vec![styled_text_box("x2", |r| {
            r.char_spacing_pt = 0.0;
            r.baseline = 0.0;
        })]))
        .pdf;
        assert_eq!(plain, zero, "显式 0 不改变字节");
        let sup = render(&one_slide(vec![styled_text_box("x2", |r| {
            r.baseline = 0.30
        })]))
        .pdf;
        assert_ne!(plain, sup, "上标改变排版");
        let wide = render(&one_slide(vec![styled_text_box("x2", |r| {
            r.char_spacing_pt = 2.0;
        })]))
        .pdf;
        assert_ne!(plain, wide, "字符间距改变排版");
    }

    /// `cap=small` 近似为全大写渲染,降级告警整篇只一条;`cap=all` 不告警。
    #[test]
    fn small_caps_warns_exactly_once() {
        use ppt_core::style::Caps;
        let small = || styled_text_box("Small caps", |r| r.cap = Caps::Small);
        let out = render(&one_slide(vec![small(), small()]));
        let n = out
            .warnings
            .iter()
            .filter(|w| matches!(w, ExportWarning::Custom { kind, .. } if kind == "small-caps"))
            .count();
        assert_eq!(n, 1, "small-caps 降级告警应恰好一条");
        let all = render(&one_slide(vec![styled_text_box("All caps", |r| {
            r.cap = Caps::All;
        })]));
        assert!(!has_custom_warning(&all, "small-caps"));
    }

    /// 公式 run(`RunKind::Math`)按普通文本画,降级告警整篇只一条;无公式不告警。
    #[test]
    fn math_run_warns_exactly_once() {
        let math = || styled_text_box("x^2", |r| r.kind = RunKind::Math);
        let out = render(&one_slide(vec![math(), math()]));
        let n = out
            .warnings
            .iter()
            .filter(|w| matches!(w, ExportWarning::Custom { kind, .. } if kind == "math-text"))
            .count();
        assert_eq!(n, 1, "math-text 降级告警应恰好一条");
        assert!(out.pdf.starts_with(b"%PDF"));
        let plain = render(&one_slide(vec![styled_text_box("x^2", |_| {})]));
        assert!(!has_custom_warning(&plain, "math-text"));
    }

    /// Task 4:两个纵排文本框仍水平降级渲染,但降级告警**恰好一条**(一次性)。
    #[test]
    fn vertical_text_warns_exactly_once() {
        let vtext = || {
            ResolvedShape::TextBox(ResolvedTextFrame {
                rect: Some(Rect::new(0, 0, 4_000_000, 3_000_000)),
                xfrm: Xfrm::default(),
                body: ppt_core::resolved::ResolvedBodyProps {
                    vertical: true,
                    ..Default::default()
                },
                paragraphs: vec![para("纵排")],
            })
        };
        let out = render(&one_slide(vec![vtext(), vtext()]));
        let n = out
            .warnings
            .iter()
            .filter(|w| matches!(w, ExportWarning::Custom { kind, .. } if kind == "vertical-text"))
            .count();
        assert_eq!(n, 1, "纵排降级告警应恰好一条(一次性)");
    }

    /// 把真实 Liberation Serif TTF 的族名等长改写为独有的 "Zephyrmark Serif",
    /// 造出一个 bundle/系统里都不存在的字体。改写同时覆盖 name 表的 ASCII 与
    /// UTF-16BE 记录,长度不变故 offset/length 保持有效,ttf-parser 仍能解析。
    fn rename_liberation(orig: &[u8]) -> Vec<u8> {
        let mut out = orig.to_vec();
        let replace_all = |buf: &mut Vec<u8>, pat: &[u8], rep: &[u8]| {
            assert_eq!(pat.len(), rep.len());
            let mut i = 0;
            while i + pat.len() <= buf.len() {
                if &buf[i..i + pat.len()] == pat {
                    buf[i..i + pat.len()].copy_from_slice(rep);
                    i += pat.len();
                } else {
                    i += 1;
                }
            }
        };
        let utf16be =
            |s: &str| -> Vec<u8> { s.encode_utf16().flat_map(u16::to_be_bytes).collect() };
        replace_all(&mut out, b"Liberation", b"Zephyrmark"); // 各 10 字节
        replace_all(&mut out, &utf16be("Liberation"), &utf16be("Zephyrmark"));
        out
    }

    /// B-11 / Task 3:`apply_font_map` **文件路径分支**真的生效。
    ///
    /// 用真实 TTF(pdf-fonts 内置 Liberation Serif,族名改写成独有的
    /// "Zephyrmark Serif")落到临时文件,证明:对照组(无 font_map)因该族缺失而
    /// `FontSubstituted` 降级;实验组(font_map 指向该文件,走文件分支
    /// `add_font_data`)请求直接命中——**无**该族的 `FontSubstituted`、PDF 里嵌入了
    /// 注入字体的 PostScript 名、且与对照组字节不同。
    ///
    /// 备注:pdf-typeset 的 `with_system_fonts` **内置**全部 12 个 Liberation 面
    /// (Sans/Serif/Mono),故原名 "Liberation Serif" 在任何机器上都可解析、注入同名
    /// 字体是无差别 no-op;改写族名后才能干净地隔离出"文件注入前后"的差异,与本机
    /// 是否装了某字体无关。
    #[test]
    fn font_map_file_branch_injects_and_hits() {
        use pdf_fonts::liberation::{liberation_face, LiberationFamily};

        let orig = liberation_face(LiberationFamily::Serif, false, false);
        let renamed = rename_liberation(orig);
        // 注入字体的自身 PostScript 名(去空格)——将出现在 PDF 的 FontDescriptor。
        let ps_name = pdf_fonts::Font::from_program(&renamed, "x")
            .expect("renamed face parses")
            .name()
            .replace(' ', ""); // "Zephyrmark Serif Regular" → "ZephyrmarkSerifRegular"
        assert!(
            ps_name.starts_with("ZephyrmarkSerif"),
            "renamed name: {ps_name}"
        );

        let path =
            std::env::temp_dir().join(format!("ppt_render_fontmap_{}.ttf", std::process::id()));
        std::fs::write(&path, &renamed).expect("write temp ttf");

        // 请求独有族名 "Zephyrmark Serif" 的单文本框(拉丁文本走 run.font)。
        let mk = || {
            let mut r = run("Serif Body Text");
            r.font = Some("Zephyrmark Serif".into());
            let p = ResolvedParagraph {
                level: 0,
                align: None,
                mar_l: None,
                indent: None,
                ln_spc: None,
                spc_bef: None,
                spc_aft: None,
                bullet: ResolvedBullet::None,
                runs: vec![r],
            };
            one_slide(vec![ResolvedShape::TextBox(ResolvedTextFrame {
                rect: Some(Rect::new(914_400, 914_400, 4_572_000, 1_828_800)),
                xfrm: Xfrm::default(),
                body: ppt_core::resolved::ResolvedBodyProps::default(),
                paragraphs: vec![p],
            })])
        };

        // 对照组:无 font_map → 族缺失 → FontSubstituted 降级(requested 含该族名)。
        let control = render(&mk());
        assert!(
            control.warnings.iter().any(|w| matches!(
                w,
                ExportWarning::FontSubstituted { requested, .. }
                    if requested == "Zephyrmark Serif"
            )),
            "对照组应对缺失族发 FontSubstituted 降级,实测: {:?}",
            control.warnings
        );

        // 实验组:font_map 指向真实文件 → 走文件分支 add_font_data → 请求命中。
        let mut font_map = BTreeMap::new();
        font_map.insert(
            "Zephyrmark Serif".to_string(),
            path.to_string_lossy().into_owned(),
        );
        let exp = render_pdf(&mk(), &BTreeMap::new(), &RenderOptions { font_map })
            .expect("render with font_map");

        // 1) 注入后不再对该族降级。
        assert!(
            !exp.warnings.iter().any(|w| matches!(
                w,
                ExportWarning::FontSubstituted { requested, .. }
                    if requested == "Zephyrmark Serif"
            )),
            "实验组不应对已注入族发 FontSubstituted,实测: {:?}",
            exp.warnings
        );
        // 2) PDF 嵌入了注入字体的 PostScript 名(硬证据:文件里的面真被用来排版)。
        assert!(
            String::from_utf8_lossy(&exp.pdf).contains("ZephyrmarkSerif"),
            "实验组 PDF 应含注入字体的 PostScript 名 ZephyrmarkSerif"
        );
        // 3) 文件注入改变了输出(与对照组的 Liberation Sans 兜底不同字节)。
        assert_ne!(exp.pdf, control.pdf, "文件分支注入应改变 PDF 字节");

        let _ = std::fs::remove_file(&path);
    }

    /// 带旋转的组合退回 `Op::Group { transform }`:内容流出现 `cm`,且与不旋转不同。
    #[test]
    fn rotated_group_emits_transform_group() {
        let group = |rot| {
            one_slide(vec![ResolvedShape::Group(ResolvedGroup {
                rect: Some(Rect::new(914_400, 914_400, 1_828_800, 1_828_800)),
                child_rect: Some(Rect::new(0, 0, 1_828_800, 1_828_800)),
                xfrm: Xfrm {
                    rot,
                    flip_h: false,
                    flip_v: false,
                },
                children: vec![auto_shape(
                    Rect::new(0, 0, 914_400, 1_828_800),
                    "rect",
                    vec![],
                )],
            })])
        };
        let plain = render(&group(0));
        let rotated = render(&group(90 * 60_000));
        assert!(String::from_utf8_lossy(&rotated.pdf).contains(" cm"));
        assert_ne!(plain.pdf, rotated.pdf);
    }

    // ---- 图表矢量渲染 ---------------------------------------------------------

    /// 一个带图表缓存数据的占位(`kind` 为 chart uri)。
    fn chart_shape(chart: Option<ppt_core::model::Chart>, uri: &str) -> ResolvedShape {
        ResolvedShape::Placeholder(ppt_core::model::GraphicPlaceholder {
            rect: Some(Rect::new(914_400, 914_400, 6_400_800, 4_572_000)),
            kind: Some(uri.to_string()),
            chart_rel_id: Some("rId2".into()),
            chart,
            diagram_rel_id: None,
            diagram_text: Vec::new(),
        })
    }

    const CHART_URI: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

    fn sample_chart(kind: ppt_core::model::ChartKind) -> ppt_core::model::Chart {
        use ppt_core::model::{Chart, ChartSeries};
        let s = |name: &str, v: [f64; 3]| ChartSeries {
            name: Some(name.into()),
            values: v.iter().map(|x| Some(*x)).collect(),
            ..ChartSeries::default()
        };
        Chart {
            kind,
            title: Some("Sales".into()),
            categories: vec!["Q1".into(), "Q2".into(), "Q3".into()],
            series: vec![
                s("North", [10.0, 20.0, 30.0]),
                s("South", [5.0, 15.0, 25.0]),
            ],
            bar_dir: Some("col".into()),
            grouping: Some("clustered".into()),
            three_d: false,
            combo: false,
            of_pie: false,
            warnings: Vec::new(),
        }
    }

    /// 占位框灰底(`PLACEHOLDER_FILL` 0.85 灰)是否出现在内容流里。
    fn has_placeholder_box(pdf: &[u8]) -> bool {
        String::from_utf8_lossy(pdf).contains("0.85 0.85 0.85 rg")
    }

    fn chart_degraded(out: &ExportResult) -> usize {
        out.warnings
            .iter()
            .filter(|w| matches!(w, ExportWarning::Custom { kind, .. } if kind == "chart-degraded"))
            .count()
    }

    /// 柱形 / 折线 / 饼图不再画占位框、不告警,而是用主题 accent 配色画出矢量图 + 文字。
    #[test]
    fn supported_charts_render_vectors_instead_of_placeholder() {
        use ppt_core::model::ChartKind;
        for kind in [ChartKind::Bar, ChartKind::Line, ChartKind::Pie] {
            let mut slide = one_slide(vec![chart_shape(
                Some(sample_chart(kind.clone())),
                CHART_URI,
            )]);
            // 自定义 accent1 为纯红,验证配色取自 slide 主题而非写死。
            slide.slides[0].accents[0] = [255, 0, 0];
            let out = render(&slide);
            let hay = String::from_utf8_lossy(&out.pdf);
            assert!(!has_placeholder_box(&out.pdf), "{kind:?}: 不应再画占位框");
            assert_eq!(chart_degraded(&out), 0, "{kind:?}: {:?}", out.warnings);
            assert!(hay.contains("1 0 0 rg"), "{kind:?}: 系列 1 用 accent1 填充");
            assert!(hay.contains("BT"), "{kind:?}: 标题 / 标签 / 图例文字");
            // 饼图扇形 = 三次贝塞尔弧(`c`);柱形全是矩形 / 直线,无曲线。
            match kind {
                ChartKind::Pie => assert!(hay.contains(" c\n"), "饼图扇形用贝塞尔弧"),
                ChartKind::Bar => assert!(!hay.contains(" c\n"), "柱形无曲线"),
                _ => {}
            }
        }
    }

    /// 不支持的类型(面积)、3D、组合、无数据、图表部件缺失 → 仍画占位框并记 `chart-degraded`;
    /// 非图表占位(SmartArt)保持旧行为:占位框、不告警。
    #[test]
    fn unsupported_charts_keep_placeholder_and_warn() {
        use ppt_core::model::ChartKind;
        let mut three_d = sample_chart(ChartKind::Bar);
        three_d.three_d = true;
        let mut combo = sample_chart(ChartKind::Bar);
        combo.combo = true;
        let mut empty = sample_chart(ChartKind::Line);
        empty.series.clear();
        for (what, shape) in [
            (
                "area",
                chart_shape(Some(sample_chart(ChartKind::Area)), CHART_URI),
            ),
            ("3d", chart_shape(Some(three_d), CHART_URI)),
            ("combo", chart_shape(Some(combo), CHART_URI)),
            ("empty", chart_shape(Some(empty), CHART_URI)),
            ("missing part", chart_shape(None, CHART_URI)),
        ] {
            let out = render(&one_slide(vec![shape]));
            assert!(has_placeholder_box(&out.pdf), "{what}: 占位框");
            assert_eq!(chart_degraded(&out), 1, "{what}: {:?}", out.warnings);
        }
        let smart_art = chart_shape(
            None,
            "http://schemas.openxmlformats.org/drawingml/2006/diagram",
        );
        let out = render(&one_slide(vec![smart_art]));
        assert!(has_placeholder_box(&out.pdf));
        assert_eq!(chart_degraded(&out), 0, "SmartArt 不发图表告警");
    }

    /// SmartArt 占位(无 drawing 部件:文字只进模型 / 导出)画占位框并记 `smartart-degraded`
    /// (按种类只发一次由上层去重;渲染侧每个占位一条)。
    #[test]
    fn smartart_placeholder_keeps_box_and_warns() {
        let uri = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
        for text in [vec![], vec!["Alpha".to_string()]] {
            let ResolvedShape::Placeholder(mut gp) = chart_shape(None, uri) else {
                unreachable!()
            };
            gp.diagram_text = text;
            let out = render(&one_slide(vec![ResolvedShape::Placeholder(gp)]));
            assert!(has_placeholder_box(&out.pdf));
            let n = out
                .warnings
                .iter()
                .filter(|w| matches!(w, ExportWarning::Custom { kind, .. } if kind == "smartart-degraded"))
                .count();
            assert_eq!(n, 1, "{:?}", out.warnings);
        }
    }

    /// 无预览图的 OLE 占位(有预览图的在解析层已成图片)画占位框并记 `ole-degraded`(渲染侧每个
    /// 占位一条);别的占位(未知种类)不误报。
    #[test]
    fn ole_placeholder_keeps_box_and_warns() {
        let uri = "http://schemas.openxmlformats.org/presentationml/2006/ole";
        let out = render(&one_slide(vec![chart_shape(None, uri)]));
        assert!(has_placeholder_box(&out.pdf));
        let n = out
            .warnings
            .iter()
            .filter(|w| matches!(w, ExportWarning::Custom { kind, .. } if kind == "ole-degraded"))
            .count();
        assert_eq!(n, 1, "{:?}", out.warnings);
        assert_eq!(chart_degraded(&out), 0, "OLE 不发图表告警");
        let other = render(&one_slide(vec![chart_shape(None, "urn:unknown")]));
        assert!(!has_custom_warning(&other, "ole-degraded"));
    }

    /// 复合饼(`ofPieChart`)不再按普通饼画:占位框 + `chart-degraded`。
    #[test]
    fn of_pie_chart_keeps_placeholder_and_warns() {
        use ppt_core::model::ChartKind;
        let mut c = sample_chart(ChartKind::Pie);
        c.of_pie = true;
        let out = render(&one_slide(vec![chart_shape(Some(c), CHART_URI)]));
        assert!(has_placeholder_box(&out.pdf));
        assert_eq!(chart_degraded(&out), 1, "{:?}", out.warnings);
    }

    /// 系列自带色胜过 accent(accent1 仍是红,系列 0 显式绿);数据标签多出文字;逐字节确定。
    #[test]
    fn chart_uses_series_colors_and_draws_data_labels() {
        use ppt_core::color::ColorSpec;
        use ppt_core::model::{ChartKind, DataLabels};
        for kind in [ChartKind::Bar, ChartKind::Line, ChartKind::Pie] {
            let plain = sample_chart(kind.clone());
            let mut styled = plain.clone();
            styled.series[0].color = Some(ColorSpec::srgb([0, 255, 0]));
            styled.series[0].point_colors = vec![(0, ColorSpec::srgb([0, 255, 0]))];
            styled.series[0].labels = Some(DataLabels {
                show_val: true,
                ..DataLabels::default()
            });
            let deck = |c| {
                let mut d = one_slide(vec![chart_shape(Some(c), CHART_URI)]);
                d.slides[0].accents[0] = [255, 0, 0];
                d
            };
            let (a, b) = (render(&deck(plain)), render(&deck(styled.clone())));
            let (ha, hb) = (
                String::from_utf8_lossy(&a.pdf),
                String::from_utf8_lossy(&b.pdf),
            );
            assert!(!ha.contains("0 1 0 rg"), "{kind:?}: 无显式色时不出现绿");
            assert!(hb.contains("0 1 0 rg"), "{kind:?}: 显式系列色");
            assert!(
                hb.matches("BT").count() > ha.matches("BT").count(),
                "{kind:?}: 数据标签多出文字"
            );
            assert_eq!(chart_degraded(&b), 0);
            assert_eq!(b.pdf, render(&deck(styled)).pdf, "{kind:?}: 确定性");
        }
    }

    /// 同输入两次渲染字节一致(无随机 / 时间 / 哈希序依赖)。
    #[test]
    fn chart_render_is_deterministic() {
        use ppt_core::model::ChartKind;
        let deck = one_slide(vec![
            chart_shape(Some(sample_chart(ChartKind::Bar)), CHART_URI),
            chart_shape(Some(sample_chart(ChartKind::Line)), CHART_URI),
            chart_shape(Some(sample_chart(ChartKind::Pie)), CHART_URI),
        ]);
        assert_eq!(render(&deck).pdf, render(&deck).pdf);
    }
}
