//! 继承链解析器(PRD-PDF-EXPORT §4,B-9):把 [`ParsedPptx`] 解析成
//! [`ResolvedPresentation`] 终态 IR。
//!
//! 每张 slide 沿 slide → slideLayout → slideMaster → theme 链:
//! - **占位符匹配**:先 idx + 等价类,再显式 idx,最后 type 等价类
//!   (`title ↔ ctrTitle`;`body ↔ subTitle ↔ obj`;`dt`/`ftr`/`sldNum` 按类;
//!   layout → master 兜底 type-only);
//! - **几何**:链上第一个 `xfrm` 整体获胜(不逐字段合并,匹配 PowerPoint);
//! - **文本样式**:master `txStyles` 桶 → master 占位符 `lstStyle` → layout 占位符
//!   `lstStyle` → slide `txBody` `lstStyle` → 段落 `pPr`/`defRPr` → run `rPr`,
//!   按段落层级取层、逐属性后者获胜;非占位符文本框用 master `otherStyle` +
//!   `p:defaultTextStyle` 作基底(presentation 级缺省视为更近的文档缺省,后者获胜);
//! - **颜色**:`schemeClr` 先经 clrMapOvr(slide → layout)/ clrMap(master)重映射,
//!   再取 `clrScheme` 终端 RGB,最后按文档顺序应用修饰变换(B-8);
//! - **字体**:`+mj-lt`/`+mn-lt` 等主题引用展开;链上全缺落 `p:style > a:fontRef`;
//! - **形状样式**:`p:style` fillRef/lnRef 经主题 `fmtScheme` 纯色解析
//!   (`phClr` 以引用色替换;非纯色项降级为代表色)。

use std::collections::BTreeMap;

use ppt_core::color::{apply_transforms, ColorSpec, ResolvedColor};
use ppt_core::geom::Rect;
use ppt_core::model::{
    AutoShape, Autofit, Background, BodyProps, Cell, Connector, Fill, GraphicPlaceholder,
    Paragraph, Presentation, Shape, Slide, Stroke, Table, TableFlags, TablePartStyle, TableStyle,
    TextFrame, TextRun,
};
use ppt_core::model::{LineEnd, LineEndKind};
use ppt_core::resolved::{
    ResolvedAnchor, ResolvedAutoShape, ResolvedBackground, ResolvedBodyProps, ResolvedBullet,
    ResolvedCell, ResolvedCellBorders, ResolvedConnector, ResolvedFill, ResolvedGroup,
    ResolvedParagraph, ResolvedPresentation, ResolvedRow, ResolvedRun, ResolvedShape,
    ResolvedSlide, ResolvedStroke, ResolvedTable, ResolvedTextFrame, DEFAULT_ACCENTS,
    DEFAULT_FONT_SIZE_PT, DEFAULT_INSET_LR_EMU, DEFAULT_INSET_TB_EMU,
};
use ppt_core::style::{
    Bullet, FontRef, PlaceholderRef, RunStyle, ShapeStyle, TextLevelStyle, TextStyleLevels,
    TxStyles,
};
use ppt_core::theme::{ClrMap, FontSet, Theme};

use crate::{InheritanceParts, LayoutPart, ParsedPptx};

/// 把解析输出整体解析成终态 IR。纯函数、绝不 panic;缺失的链级按缺省兜底。
pub fn resolve(parsed: &ParsedPptx) -> ResolvedPresentation {
    resolve_parts(&parsed.presentation, &parsed.inherit)
}

/// 同 [`resolve`],但接受拆开的两部分(py-bindings 各自持有 `Arc` 时无需重组克隆)。
pub fn resolve_parts(
    presentation: &Presentation,
    inherit: &InheritanceParts,
) -> ResolvedPresentation {
    ResolvedPresentation {
        slide_size: presentation.slide_size,
        slides: presentation
            .slides
            .iter()
            .map(|s| resolve_slide(s, inherit))
            .collect(),
    }
}

/// 单张 slide 的解析上下文(链上各级的只读视图)。
struct Ctx<'a> {
    theme: Option<&'a Theme>,
    clr_map: ClrMap,
    layout_shapes: &'a [Shape],
    master_shapes: &'a [Shape],
    tx_styles: Option<&'a TxStyles>,
    default_text_style: Option<&'a TextStyleLevels>,
    table_styles: &'a BTreeMap<String, TableStyle>,
}

fn resolve_slide(slide: &Slide, inherit: &InheritanceParts) -> ResolvedSlide {
    let layout = slide
        .layout_name
        .as_deref()
        .and_then(|n| inherit.layouts.get(n));
    let master = layout
        .and_then(|l| l.master_name.as_deref())
        .and_then(|n| inherit.masters.get(n));
    let theme = master
        .and_then(|m| m.theme_name.as_deref())
        .and_then(|n| inherit.themes.get(n));
    // clrMap 链:slide 覆盖 → layout 覆盖 → master 本映射 → 惯例缺省。
    let clr_map = slide
        .clr_map_ovr
        .clone()
        .or_else(|| layout.and_then(|l| l.clr_map_ovr.clone()))
        .or_else(|| master.and_then(|m| m.clr_map.clone()))
        .unwrap_or_default();
    let ctx = Ctx {
        theme,
        clr_map,
        layout_shapes: layout.map(|l| l.shapes.as_slice()).unwrap_or(&[]),
        master_shapes: master.map(|m| m.shapes.as_slice()).unwrap_or(&[]),
        tx_styles: master.and_then(|m| m.tx_styles.as_ref()),
        default_text_style: inherit.default_text_style.as_ref(),
        table_styles: &inherit.table_styles,
    };
    // B-10:背景继承链 slide → layout → master(第一个存在的赢,不逐字段合并)。
    let background = slide
        .background
        .as_ref()
        .or_else(|| layout.and_then(|l| l.background.as_ref()))
        .or_else(|| master.and_then(|m| m.background.as_ref()));
    ResolvedSlide {
        index: slide.index,
        background: resolve_background(background, &ctx),
        inherited_shapes: resolve_inherited(slide, layout, &ctx),
        accents: theme_accents(&ctx),
        shapes: slide
            .shapes
            .iter()
            .map(|sh| resolve_shape(sh, &ctx))
            .collect(),
    }
}

/// master / layout 上的非占位符形状(绘制顺序:master → layout,层内文档顺序)。
///
/// `showMasterSp`(ECMA-376 §19.3.1.38 `sld` / §19.3.1.39 `sldLayout`):
/// - slide 设 `0`:不画任何继承图形(layout 与 master 都隐藏,= PowerPoint「隐藏背景图形」);
/// - layout 设 `0`:只隐藏 master 图形,layout 自身图形照画。
///
/// 占位符(`p:ph`)只是模板,从不画。非占位符形状无占位符链,文字走 master
/// `otherStyle` + `defaultTextStyle` 基链(与 slide 上的非占位符文本框同口径);
/// 颜色沿用本 slide 的有效 clrMap(PowerPoint 按所在 slide 的映射着色继承图形)。
fn resolve_inherited(slide: &Slide, layout: Option<&LayoutPart>, ctx: &Ctx) -> Vec<ResolvedShape> {
    if !slide.show_master_sp {
        return Vec::new();
    }
    let show_master = layout.and_then(|l| l.show_master_sp).unwrap_or(true);
    let master_shapes = if show_master { ctx.master_shapes } else { &[] };
    master_shapes
        .iter()
        .chain(ctx.layout_shapes)
        .filter(|sh| ph_of(sh).is_none())
        .map(|sh| resolve_shape(sh, ctx))
        .collect()
}

/// B-10:`p:bg` → 终态背景(纯色 / 图片 / 主题引用降级为代表色)。
fn resolve_background(bg: Option<&Background>, ctx: &Ctx) -> Option<ResolvedBackground> {
    match bg? {
        Background::Fill(f) => resolve_fill(ctx, Some(f), None).map(ResolvedBackground::Color),
        Background::Blip { media_name } => media_name
            .clone()
            .map(|m| ResolvedBackground::Picture { media_name: m }),
        Background::Ref { color, .. } => color
            .as_ref()
            .map(|c| ResolvedBackground::Color(ResolvedFill::Solid(resolve_color(ctx, c, None)))),
    }
}

/// B-6:`bodyPr` 占位符继承链(master → layout → 形状自身)合并后回填 OOXML 缺省。
fn resolve_body(
    own: &BodyProps,
    layout_ph: Option<&Shape>,
    master_ph: Option<&Shape>,
) -> ResolvedBodyProps {
    let mut merged = master_ph.and_then(shape_body).cloned().unwrap_or_default();
    if let Some(lb) = layout_ph.and_then(shape_body) {
        merged = merged.overridden_by(lb);
    }
    merged = merged.overridden_by(own);
    to_resolved_body(&merged)
}

/// 占位符形状的 `bodyPr`(仅文本承载形状有)。
fn shape_body(sh: &Shape) -> Option<&BodyProps> {
    match sh {
        Shape::TextBox(tf) => Some(&tf.body),
        Shape::Auto(a) => a.text.as_ref().map(|tf| &tf.body),
        _ => None,
    }
}

/// 合并后的 `BodyProps`(全 Option)→ 终态(OOXML 缺省已回填)。
fn to_resolved_body(b: &BodyProps) -> ResolvedBodyProps {
    let (font_scale, ln_spc_reduction) = match b.autofit {
        Some(Autofit::Normal {
            font_scale,
            ln_spc_reduction,
        }) => (
            font_scale.map(|v| v as f32 / 100_000.0),
            ln_spc_reduction.map(|v| v as f32 / 100_000.0),
        ),
        _ => (None, None),
    };
    ResolvedBodyProps {
        anchor: ResolvedAnchor::from_ooxml(b.anchor.as_deref()),
        anchor_ctr: b.anchor_ctr.unwrap_or(false),
        l_ins: b.l_ins.unwrap_or(DEFAULT_INSET_LR_EMU),
        t_ins: b.t_ins.unwrap_or(DEFAULT_INSET_TB_EMU),
        r_ins: b.r_ins.unwrap_or(DEFAULT_INSET_LR_EMU),
        b_ins: b.b_ins.unwrap_or(DEFAULT_INSET_TB_EMU),
        wrap: b.wrap.unwrap_or(true),
        font_scale,
        ln_spc_reduction,
        autofit_normal: matches!(b.autofit, Some(Autofit::Normal { .. })),
        // v1:不裁剪(引擎字形软剪裁已修但保守放行溢出;normAutofit 的 fontScale
        // 已把文字缩小,无需再裁)。
        clip: false,
        vertical: b.vert.as_deref().map(|v| v != "horz").unwrap_or(false),
    }
}

fn resolve_shape(shape: &Shape, ctx: &Ctx) -> ResolvedShape {
    match shape {
        Shape::TextBox(tf) => ResolvedShape::TextBox(resolve_text_box(tf, ctx)),
        Shape::Auto(a) => ResolvedShape::Auto(resolve_auto(a, ctx)),
        Shape::Connector(c) => ResolvedShape::Connector(resolve_connector(c, ctx)),
        Shape::Table(t) => ResolvedShape::Table(resolve_table(t, ctx)),
        Shape::Picture(p) => {
            // 占位符几何物化;其余原样(裁剪 / 拉伸属 B-4)。
            let (layout_ph, master_ph) = find_chain(ctx, p.placeholder.as_ref());
            let rect = p
                .rect
                .or_else(|| layout_ph.and_then(shape_rect))
                .or_else(|| master_ph.and_then(shape_rect));
            let mut pic = p.clone();
            pic.rect = rect;
            ResolvedShape::Picture(pic)
        }
        Shape::Group(g) => {
            // 变换与子坐标空间原样透传;仿射累积交渲染侧(B-5)。
            ResolvedShape::Group(ResolvedGroup {
                rect: g.rect,
                child_rect: g.child_rect,
                xfrm: g.xfrm,
                children: g.children.iter().map(|c| resolve_shape(c, ctx)).collect(),
            })
        }
        Shape::Placeholder(gp) => ResolvedShape::Placeholder(resolve_graphic(gp, ctx)),
    }
}

/// 图表帧:系列色 / 逐点色里的 schemeClr(及变换)经 clrMap + clrScheme 终端化为显式 srgb
/// (不带变换),渲染侧直接取色;alpha 丢弃(图表按不透明画)。
fn resolve_graphic(gp: &GraphicPlaceholder, ctx: &Ctx) -> GraphicPlaceholder {
    let mut gp = gp.clone();
    let terminal = |spec: &ColorSpec| ColorSpec::srgb(resolve_color(ctx, spec, None).rgb);
    for s in gp.chart.iter_mut().flat_map(|c| c.series.iter_mut()) {
        s.color = s.color.as_ref().map(terminal);
        for (_, c) in &mut s.point_colors {
            *c = terminal(c);
        }
    }
    gp
}

// ---- 文本 -----------------------------------------------------------------

fn resolve_text_box(tf: &TextFrame, ctx: &Ctx) -> ResolvedTextFrame {
    let ph = tf.placeholder.as_ref();
    let (layout_ph, master_ph) = find_chain(ctx, ph);
    // 几何:链上第一个 xfrm 整体获胜。
    let rect = tf
        .rect
        .or_else(|| layout_ph.and_then(shape_rect))
        .or_else(|| master_ph.and_then(shape_rect));
    let chain = style_chain(ctx, ph, tf.list_style.as_ref(), layout_ph, master_ph);
    let font_ref = tf.style.as_ref().and_then(|s| s.font_ref.as_ref());
    ResolvedTextFrame {
        rect,
        xfrm: tf.xfrm,
        body: resolve_body(&tf.body, layout_ph, master_ph),
        paragraphs: tf
            .paragraphs
            .iter()
            .map(|p| resolve_paragraph(p, &chain, font_ref, ctx))
            .collect(),
    }
}

fn resolve_paragraph(
    para: &Paragraph,
    chain: &[&TextStyleLevels],
    font_ref: Option<&FontRef>,
    ctx: &Ctx,
) -> ResolvedParagraph {
    // 层级样式逐级合并(远 → 近),最后叠段落直接 pPr。
    let mut merged = TextLevelStyle::default();
    for ls in chain {
        if let Some(level) = ls.level(para.level) {
            merged = merged.overridden_by(level);
        }
    }
    merged = merged.overridden_by(&para.props);
    let base_rpr = merged.def_rpr.clone().unwrap_or_default();
    ResolvedParagraph {
        level: para.level,
        align: merged.align.clone(),
        mar_l: merged.mar_l,
        indent: merged.indent,
        ln_spc: merged.ln_spc,
        spc_bef: merged.spc_bef,
        spc_aft: merged.spc_aft,
        bullet: resolve_bullet(&merged, ctx),
        runs: para
            .runs
            .iter()
            .map(|r| resolve_run(r, &base_rpr, font_ref, ctx))
            .collect(),
    }
}

fn resolve_bullet(merged: &TextLevelStyle, ctx: &Ctx) -> ResolvedBullet {
    let font = merged.bu_font.as_deref().and_then(|f| resolve_font(ctx, f));
    let size_pct = merged.bu_size_pct.map(|v| v as f32 / 100_000.0);
    match &merged.bullet {
        // 未指定与显式 buNone 的终态一致:无符号。
        None | Some(Bullet::None) => ResolvedBullet::None,
        Some(Bullet::Char(ch)) => ResolvedBullet::Char {
            ch: ch.clone(),
            font,
            size_pct,
        },
        Some(Bullet::AutoNum { scheme, start_at }) => ResolvedBullet::AutoNum {
            scheme: scheme.clone(),
            start_at: *start_at,
            font,
            size_pct,
        },
    }
}

fn resolve_run(
    run: &TextRun,
    base: &RunStyle,
    font_ref: Option<&FontRef>,
    ctx: &Ctx,
) -> ResolvedRun {
    // run 直接格式化永远最后获胜。
    let direct = RunStyle {
        size_pt: run.size_pt,
        bold: run.bold,
        italic: run.italic,
        underline: run.underline,
        strike: run.strike,
        font: run.font.clone(),
        ea_font: run.ea_font.clone(),
        cs_font: run.cs_font.clone(),
        color: run.color.clone(),
        char_spacing_pt: run.char_spacing_pt,
        baseline: run.baseline,
        cap: run.cap,
    };
    let m = base.overridden_by(&direct);
    // 字体:主题引用展开;链上全缺落 `p:style > a:fontRef` 的 major/minor。
    let fr_set = font_ref_set(ctx, font_ref);
    let font = m
        .font
        .as_deref()
        .and_then(|f| resolve_font(ctx, f))
        .or_else(|| fr_set.and_then(|s| s.latin.clone()));
    let ea_font = m
        .ea_font
        .as_deref()
        .and_then(|f| resolve_font(ctx, f))
        .or_else(|| fr_set.and_then(|s| s.ea.clone()));
    let cs_font = m
        .cs_font
        .as_deref()
        .and_then(|f| resolve_font(ctx, f))
        .or_else(|| fr_set.and_then(|s| s.cs.clone()));
    // 颜色:链上全缺落 fontRef 子颜色,再兜底黑。
    let color = m
        .color
        .as_ref()
        .map(|c| resolve_color(ctx, c, None))
        .or_else(|| {
            font_ref
                .and_then(|fr| fr.color.as_ref())
                .map(|c| resolve_color(ctx, c, None))
        })
        .unwrap_or(ResolvedColor::opaque([0, 0, 0]));
    ResolvedRun {
        text: run.text.clone(),
        kind: run.kind.clone(),
        font,
        ea_font,
        cs_font,
        size_pt: m.size_pt.unwrap_or(DEFAULT_FONT_SIZE_PT),
        bold: m.bold.unwrap_or(false),
        italic: m.italic.unwrap_or(false),
        underline: m.underline.unwrap_or(false),
        strike: m.strike.unwrap_or(false),
        color,
        char_spacing_pt: m.char_spacing_pt.unwrap_or(0.0),
        baseline: m.baseline.unwrap_or(0.0),
        cap: m.cap.unwrap_or_default(),
    }
}

// ---- 形状 -----------------------------------------------------------------

fn resolve_auto(a: &AutoShape, ctx: &Ctx) -> ResolvedAutoShape {
    let ph = a.placeholder.as_ref();
    let (layout_ph, master_ph) = find_chain(ctx, ph);
    let rect = a
        .rect
        .or_else(|| layout_ph.and_then(shape_rect))
        .or_else(|| master_ph.and_then(shape_rect));
    let text = a.text.as_ref().map(|tf| {
        let chain = style_chain(ctx, ph, tf.list_style.as_ref(), layout_ph, master_ph);
        let font_ref = a.style.as_ref().and_then(|s| s.font_ref.as_ref());
        ResolvedTextFrame {
            rect,
            // 形状上的文字随形状旋转(翻转不镜像文字)。
            xfrm: a.xfrm,
            body: resolve_body(&tf.body, layout_ph, master_ph),
            paragraphs: tf
                .paragraphs
                .iter()
                .map(|p| resolve_paragraph(p, &chain, font_ref, ctx))
                .collect(),
        }
    });
    ResolvedAutoShape {
        rect,
        xfrm: a.xfrm,
        geometry: a.geometry.clone(),
        adjusts: a.adjusts.clone(),
        fill: resolve_fill(ctx, a.fill.as_ref(), a.style.as_ref()),
        stroke: resolve_stroke(ctx, a.stroke.as_ref(), a.style.as_ref()),
        text,
        custom_geometry: a.custom_geometry,
    }
}

fn resolve_connector(c: &Connector, ctx: &Ctx) -> ResolvedConnector {
    ResolvedConnector {
        rect: c.rect,
        xfrm: c.xfrm,
        geometry: c.geometry.clone(),
        adjusts: c.adjusts.clone(),
        fill: resolve_fill(ctx, c.fill.as_ref(), c.style.as_ref()),
        stroke: resolve_stroke(ctx, c.stroke.as_ref(), c.style.as_ref()),
        no_line: c.stroke.as_ref().is_some_and(|s| s.no_fill),
        custom_geometry: c.custom_geometry,
    }
}

fn resolve_table(t: &Table, ctx: &Ctx) -> ResolvedTable {
    // 单元格文字无占位符链;用非占位符基链(otherStyle + defaultTextStyle)。
    let chain = style_chain(ctx, None, None, None, None);
    // 表格样式(`tableStyles.xml`):找不到 styleId 时退回只用显式属性。
    let style = t
        .table_style_id
        .as_deref()
        .and_then(|id| ctx.table_styles.get(id));
    let nrows = t.rows.len();
    let ncols = t
        .col_widths
        .len()
        .max(t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0));
    ResolvedTable {
        rect: t.rect,
        col_widths: t.col_widths.clone(),
        table_style_id: t.table_style_id.clone(),
        style_resolved: style.is_some(),
        rows: t
            .rows
            .iter()
            .enumerate()
            .map(|(ri, row)| ResolvedRow {
                cells: row
                    .cells
                    .iter()
                    .enumerate()
                    .map(|(ci, c)| {
                        let pos = GridPos {
                            row: ri,
                            col: ci,
                            row_span: (c.row_span.max(1) as usize).min(nrows - ri),
                            col_span: (c.col_span.max(1) as usize).min(ncols - ci),
                            nrows,
                            ncols,
                        };
                        let cs = style.map(|s| cell_table_style(s, t.flags, &pos));
                        resolve_cell(c, &chain, cs.as_ref(), ctx)
                    })
                    .collect(),
                height: row.height,
            })
            .collect(),
    }
}

/// 单元格在表格网格中的位置(跨行 / 跨列已按表格边界截断)。
struct GridPos {
    row: usize,
    col: usize,
    row_span: usize,
    col_span: usize,
    nrows: usize,
    ncols: usize,
}

/// 表格样式部件作用的区域形状:决定部件的 left/right/top/bottom 是外沿还是取
/// insideH/insideV。整表 = 全表;行部件(行带 / 首末行)= 单行;列部件 = 单列。
#[derive(Clone, Copy)]
enum PartRegion {
    Table,
    Row,
    Col,
}

/// 一个单元格叠加完毕的表格样式(终态:未指定与显式"无"已合一为 `None`)。
struct CellTableStyle {
    fill: Option<ColorSpec>,
    left: Option<Stroke>,
    right: Option<Stroke>,
    top: Option<Stroke>,
    bottom: Option<Stroke>,
    text_color: Option<ColorSpec>,
    bold: Option<bool>,
}

/// 按 OOXML 优先级叠加单元格适用的样式部件(后者逐属性覆盖前者):
/// wholeTbl < band1V/band2V < band1H/band2H < lastCol < firstCol < lastRow < firstRow。
/// 行带 / 列带计数跳过启用的首末行 / 首末列(表头行不计入行带)。
fn cell_table_style(style: &TableStyle, flags: TableFlags, pos: &GridPos) -> CellTableStyle {
    let first_row = flags.first_row && pos.row == 0;
    let last_row = flags.last_row && pos.row + pos.row_span >= pos.nrows;
    let first_col = flags.first_col && pos.col == 0;
    let last_col = flags.last_col && pos.col + pos.col_span >= pos.ncols;
    let mut parts: Vec<(&TablePartStyle, PartRegion)> = vec![(&style.whole_tbl, PartRegion::Table)];
    if flags.band_col && !first_col && !last_col {
        let band = pos.col - usize::from(flags.first_col);
        let part = if band.is_multiple_of(2) {
            &style.band1_v
        } else {
            &style.band2_v
        };
        parts.push((part, PartRegion::Col));
    }
    if flags.band_row && !first_row && !last_row {
        let band = pos.row - usize::from(flags.first_row);
        let part = if band.is_multiple_of(2) {
            &style.band1_h
        } else {
            &style.band2_h
        };
        parts.push((part, PartRegion::Row));
    }
    if last_col {
        parts.push((&style.last_col, PartRegion::Col));
    }
    if first_col {
        parts.push((&style.first_col, PartRegion::Col));
    }
    if last_row {
        parts.push((&style.last_row, PartRegion::Row));
    }
    if first_row {
        parts.push((&style.first_row, PartRegion::Row));
    }

    let at_left = pos.col == 0;
    let at_right = pos.col + pos.col_span >= pos.ncols;
    let at_top = pos.row == 0;
    let at_bottom = pos.row + pos.row_span >= pos.nrows;
    let (mut fill, mut text_color, mut bold) = (None, None, None);
    let (mut left, mut right, mut top, mut bottom) = (None, None, None, None);
    for (part, region) in parts {
        let b = &part.borders;
        // 区域外沿取对应边,区域内部取格间线(列部件左右恒为外沿,行部件上下恒为外沿)。
        let (outer_l, outer_r) = match region {
            PartRegion::Col => (true, true),
            _ => (at_left, at_right),
        };
        let (outer_t, outer_b) = match region {
            PartRegion::Row => (true, true),
            _ => (at_top, at_bottom),
        };
        let pick = |outer: bool, edge: &Option<Option<Stroke>>, inside: &Option<Option<Stroke>>| {
            if outer {
                edge.clone()
            } else {
                inside.clone()
            }
        };
        left = pick(outer_l, &b.left, &b.inside_v).or(left);
        right = pick(outer_r, &b.right, &b.inside_v).or(right);
        top = pick(outer_t, &b.top, &b.inside_h).or(top);
        bottom = pick(outer_b, &b.bottom, &b.inside_h).or(bottom);
        fill = part.fill.clone().or(fill);
        text_color = part.text_color.clone().or(text_color);
        bold = part.bold.or(bold);
    }
    CellTableStyle {
        fill: fill.flatten(),
        left: left.flatten(),
        right: right.flatten(),
        top: top.flatten(),
        bottom: bottom.flatten(),
        text_color,
        bold,
    }
}

/// 表格样式的文字色 / 粗体 → 一层各级相同的缺省 run 样式,接在非占位符基链之后
/// (段落 `pPr` / run `rPr` 的显式属性仍然更近、获胜)。
fn table_text_layer(style: &CellTableStyle) -> Option<TextStyleLevels> {
    if style.text_color.is_none() && style.bold.is_none() {
        return None;
    }
    let level = TextLevelStyle {
        def_rpr: Some(RunStyle {
            color: style.text_color.clone(),
            bold: style.bold,
            ..RunStyle::default()
        }),
        ..TextLevelStyle::default()
    };
    Some(TextStyleLevels {
        levels: Box::new(std::array::from_fn(|_| Some(level.clone()))),
    })
}

fn resolve_cell(
    cell: &Cell,
    chain: &[&TextStyleLevels],
    style: Option<&CellTableStyle>,
    ctx: &Ctx,
) -> ResolvedCell {
    // 显式 `tcPr` 逐边 / 填充获胜;缺失时取表格样式(显式 `noFill` 压制样式填充)。
    // 显式 `noFill` 边(`hidden`)不画,也不回落到样式边。
    let border = |explicit: Option<&Stroke>, hidden: bool, styled: Option<&Stroke>| {
        if hidden {
            return None;
        }
        resolve_stroke(ctx, explicit.or(styled), None)
    };
    let styled_fill = style
        .and_then(|s| s.fill.as_ref())
        .filter(|_| !cell.no_fill);
    let text_layer = style.and_then(table_text_layer);
    let mut cell_chain = chain.to_vec();
    if let Some(layer) = &text_layer {
        cell_chain.push(layer);
    }
    ResolvedCell {
        paragraphs: cell
            .paragraphs
            .iter()
            .map(|p| resolve_paragraph(p, &cell_chain, None, ctx))
            .collect(),
        col_span: cell.col_span,
        row_span: cell.row_span,
        fill: cell
            .fill
            .as_ref()
            .or(styled_fill)
            .map(|c| resolve_color(ctx, c, None)),
        merged: cell.merged,
        mar_l: cell.mar_l.unwrap_or(DEFAULT_INSET_LR_EMU),
        mar_r: cell.mar_r.unwrap_or(DEFAULT_INSET_LR_EMU),
        mar_t: cell.mar_t.unwrap_or(DEFAULT_INSET_TB_EMU),
        mar_b: cell.mar_b.unwrap_or(DEFAULT_INSET_TB_EMU),
        anchor: ResolvedAnchor::from_ooxml(cell.anchor.as_deref()),
        borders: ResolvedCellBorders {
            left: border(
                cell.borders.left.as_ref(),
                cell.borders.no_left,
                style.and_then(|s| s.left.as_ref()),
            ),
            right: border(
                cell.borders.right.as_ref(),
                cell.borders.no_right,
                style.and_then(|s| s.right.as_ref()),
            ),
            top: border(
                cell.borders.top.as_ref(),
                cell.borders.no_top,
                style.and_then(|s| s.top.as_ref()),
            ),
            bottom: border(
                cell.borders.bottom.as_ref(),
                cell.borders.no_bottom,
                style.and_then(|s| s.bottom.as_ref()),
            ),
        },
    }
}

/// 填充解析:显式 `spPr` 填充获胜(`noFill` 也是显式——直接无填充,不落
/// `fillRef`;渐变降级为首个 stop 的代表色,渲染侧据 [`ResolvedFill::Gradient`]
/// 记 `GradientDegraded`;形状级图片填充 v1 不涂)。未设置时经 `fillRef` 查主题
/// `fillStyleLst`(`phClr` 以引用色替换;非纯色 / 越界项降级为引用色本身 = 代表色)。
fn resolve_fill(
    ctx: &Ctx,
    fill: Option<&Fill>,
    style: Option<&ShapeStyle>,
) -> Option<ResolvedFill> {
    match fill {
        Some(Fill::None) => return None,
        Some(Fill::Solid(spec)) => {
            return Some(ResolvedFill::Solid(resolve_color(ctx, spec, None)))
        }
        Some(Fill::Gradient(stops)) => {
            return stops
                .first()
                .map(|s| ResolvedFill::Gradient(resolve_color(ctx, s, None)));
        }
        Some(Fill::Blip) => return None,
        None => {}
    }
    let fr = style?.fill_ref.as_ref().filter(|r| r.idx >= 1)?;
    let ph_rgb = fr
        .color
        .as_ref()
        .map(|c| resolve_color(ctx, c, None))
        .map(|c| c.rgb);
    let entry = ctx
        .theme
        .and_then(|t| t.fill_styles.get(fr.idx as usize - 1))
        .cloned()
        .flatten();
    match entry {
        Some(spec) => Some(ResolvedFill::Solid(resolve_color(ctx, &spec, ph_rgb))),
        None => ph_rgb.map(|rgb| ResolvedFill::Solid(ResolvedColor::opaque(rgb))),
    }
}

/// 描边解析:显式 `a:ln` 字段逐项获胜;缺色 / 缺宽 / 缺线端(`headEnd` / `tailEnd`)
/// 经 `lnRef` 从主题 `lnStyleLst` 补(虚线不走主题,沿旧行为)。
fn resolve_stroke(
    ctx: &Ctx,
    stroke: Option<&Stroke>,
    style: Option<&ShapeStyle>,
) -> Option<ResolvedStroke> {
    if stroke.is_some_and(|s| s.no_fill) {
        return None;
    }
    let ln_ref = style.and_then(|s| s.ln_ref.as_ref()).filter(|r| r.idx >= 1);
    let ph_rgb = ln_ref
        .and_then(|r| r.color.as_ref())
        .map(|c| resolve_color(ctx, c, None).rgb);
    let theme_line = ln_ref.and_then(|r| {
        ctx.theme
            .and_then(|t| t.line_styles.get(r.idx as usize - 1))
    });
    let color = stroke
        .and_then(|s| s.color.as_ref())
        .map(|c| resolve_color(ctx, c, None))
        .or_else(|| {
            theme_line
                .and_then(|tl| tl.color.as_ref())
                .map(|c| resolve_color(ctx, c, ph_rgb))
        })
        .or_else(|| ph_rgb.map(ResolvedColor::opaque));
    let width_emu = stroke
        .and_then(|s| s.width_emu)
        .or_else(|| theme_line.and_then(|tl| tl.width_emu));
    let dash = stroke.and_then(|s| s.dash.clone());
    let head_end = stroke
        .and_then(|s| s.head_end.clone())
        .or_else(|| theme_line.and_then(|tl| tl.head_end.clone()));
    let tail_end = stroke
        .and_then(|s| s.tail_end.clone())
        .or_else(|| theme_line.and_then(|tl| tl.tail_end.clone()));
    let has_end = |e: &Option<LineEnd>| e.as_ref().is_some_and(|e| e.kind != LineEndKind::None);
    if color.is_none()
        && width_emu.is_none()
        && dash.is_none()
        && !has_end(&head_end)
        && !has_end(&tail_end)
    {
        return None;
    }
    Some(ResolvedStroke {
        color,
        width_emu,
        dash,
        head_end,
        tail_end,
    })
}

// ---- 颜色 / 字体 -----------------------------------------------------------

/// 主题 accent1..6 终端 RGB(经 clrMap);无主题时取 Office 缺省调色板。
fn theme_accents(ctx: &Ctx) -> [[u8; 3]; 6] {
    if ctx.theme.is_none() {
        return DEFAULT_ACCENTS;
    }
    std::array::from_fn(|i| {
        let spec = ColorSpec::Scheme {
            name: format!("accent{}", i + 1),
            transforms: Vec::new(),
        };
        resolve_color(ctx, &spec, None).rgb
    })
}

/// 解析一个颜色 spec;`ph_clr` 是 `phClr`(样式引用占位色)的替换基色。
fn resolve_color(ctx: &Ctx, spec: &ColorSpec, ph_clr: Option<[u8; 3]>) -> ResolvedColor {
    match spec {
        ColorSpec::Srgb { rgb, transforms } => apply_transforms(*rgb, transforms),
        ColorSpec::Scheme { name, transforms } => {
            let base = if name == "phClr" {
                ph_clr.unwrap_or([0, 0, 0])
            } else {
                // 先经 clrMap 重映射(tx1→dk1 等),再取 clrScheme 终端 RGB。
                let slot = ctx.clr_map.map(name);
                ctx.theme
                    .and_then(|t| t.color_scheme.get(slot))
                    .map(|c| c.rgb)
                    .unwrap_or([0, 0, 0])
            };
            apply_transforms(base, transforms)
        }
    }
}

/// 字体名解析:`+mj-*`/`+mn-*` 主题引用展开;普通名字原样;未知引用 → `None`。
fn resolve_font(ctx: &Ctx, name: &str) -> Option<String> {
    if !name.starts_with('+') {
        return Some(name.to_string());
    }
    let fs = &ctx.theme?.font_scheme;
    match name {
        "+mj-lt" => fs.major.latin.clone(),
        "+mj-ea" => fs.major.ea.clone(),
        "+mj-cs" => fs.major.cs.clone(),
        "+mn-lt" => fs.minor.latin.clone(),
        "+mn-ea" => fs.minor.ea.clone(),
        "+mn-cs" => fs.minor.cs.clone(),
        _ => None,
    }
}

/// `fontRef@idx`(major / minor)对应的主题字体集合。
fn font_ref_set<'a>(ctx: &Ctx<'a>, font_ref: Option<&FontRef>) -> Option<&'a FontSet> {
    let fr = font_ref?;
    let fs = &ctx.theme?.font_scheme;
    match fr.idx.as_str() {
        "major" => Some(&fs.major),
        "minor" => Some(&fs.minor),
        _ => None,
    }
}

// ---- 占位符匹配 ------------------------------------------------------------

/// 找一个占位符在 layout / master 上的匹配(master 匹配优先以 layout 匹配到的
/// 占位符标识作 key,更贴近 PowerPoint 的逐级匹配)。
fn find_chain<'a>(
    ctx: &Ctx<'a>,
    ph: Option<&PlaceholderRef>,
) -> (Option<&'a Shape>, Option<&'a Shape>) {
    let Some(ph) = ph else {
        return (None, None);
    };
    let layout_ph = match_ph(ctx.layout_shapes, ph);
    let master_key = layout_ph.and_then(ph_of).unwrap_or(ph);
    let master_ph = match_ph(ctx.master_shapes, master_key);
    (layout_ph, master_ph)
}

/// 占位符匹配(PRD §4.1):
/// 1. idx(缺省 0)+ 等价类都同;2. 目标带显式 idx 时按 idx;3. 仅按等价类。
fn match_ph<'a>(shapes: &'a [Shape], target: &PlaceholderRef) -> Option<&'a Shape> {
    let t_idx = eff_idx(target);
    let t_class = ph_class(eff_kind(target));
    let candidates: Vec<(&Shape, &PlaceholderRef)> = shapes
        .iter()
        .filter_map(|s| ph_of(s).map(|p| (s, p)))
        .collect();
    if let Some((s, _)) = candidates
        .iter()
        .find(|(_, p)| eff_idx(p) == t_idx && ph_class(eff_kind(p)) == t_class)
    {
        return Some(s);
    }
    if target.idx.is_some() {
        if let Some((s, _)) = candidates.iter().find(|(_, p)| eff_idx(p) == t_idx) {
            return Some(s);
        }
    }
    candidates
        .iter()
        .find(|(_, p)| ph_class(eff_kind(p)) == t_class)
        .map(|(s, _)| *s)
}

/// 占位符种类的匹配等价类(`title ↔ ctrTitle`;`body ↔ subTitle ↔ obj`)。
fn ph_class(kind: &str) -> &str {
    match kind {
        "title" | "ctrTitle" => "title",
        "body" | "subTitle" | "obj" => "body",
        other => other,
    }
}

/// `type` 缺省语义为 `body`(ECMA-376 §19.3.1.36 默认值)。
fn eff_kind(ph: &PlaceholderRef) -> &str {
    ph.kind.as_deref().unwrap_or("body")
}

/// `idx` 缺省按 0 匹配。
fn eff_idx(ph: &PlaceholderRef) -> u32 {
    ph.idx.unwrap_or(0)
}

/// 形状携带的占位符标识。
fn ph_of(shape: &Shape) -> Option<&PlaceholderRef> {
    match shape {
        Shape::TextBox(tf) => tf.placeholder.as_ref(),
        Shape::Auto(a) => a.placeholder.as_ref(),
        Shape::Picture(p) => p.placeholder.as_ref(),
        _ => None,
    }
}

/// 形状自身的 xfrm 矩形(几何继承用)。
fn shape_rect(shape: &Shape) -> Option<Rect> {
    match shape {
        Shape::TextBox(tf) => tf.rect,
        Shape::Auto(a) => a.rect,
        Shape::Picture(p) => p.rect,
        _ => None,
    }
}

/// 形状携带的 `lstStyle`(文本样式继承用)。
fn shape_list_style(shape: &Shape) -> Option<&TextStyleLevels> {
    match shape {
        Shape::TextBox(tf) => tf.list_style.as_ref(),
        Shape::Auto(a) => a.text.as_ref().and_then(|tf| tf.list_style.as_ref()),
        _ => None,
    }
}

/// 组一个形状的文本样式链(远 → 近;PRD §4.1)。
fn style_chain<'b>(
    ctx: &'b Ctx<'_>,
    ph: Option<&PlaceholderRef>,
    slide_ls: Option<&'b TextStyleLevels>,
    layout_ph: Option<&'b Shape>,
    master_ph: Option<&'b Shape>,
) -> Vec<&'b TextStyleLevels> {
    let mut chain: Vec<&TextStyleLevels> = Vec::new();
    match ph {
        Some(p) => {
            if let Some(tx) = ctx.tx_styles {
                chain.push(match ph_class(eff_kind(p)) {
                    "title" => &tx.title,
                    "body" => &tx.body,
                    _ => &tx.other,
                });
            }
            if let Some(ls) = master_ph.and_then(shape_list_style) {
                chain.push(ls);
            }
            if let Some(ls) = layout_ph.and_then(shape_list_style) {
                chain.push(ls);
            }
        }
        None => {
            // 非占位符:master otherStyle + presentation defaultTextStyle 作基底。
            if let Some(tx) = ctx.tx_styles {
                chain.push(&tx.other);
            }
            if let Some(d) = ctx.default_text_style {
                chain.push(d);
            }
        }
    }
    if let Some(ls) = slide_ls {
        chain.push(ls);
    }
    chain
}
