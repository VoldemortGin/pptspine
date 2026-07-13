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

use ppt_core::color::{apply_transforms, ColorSpec, ResolvedColor};
use ppt_core::geom::Rect;
use ppt_core::model::{
    AutoShape, Autofit, Background, BodyProps, Cell, Connector, Fill, Paragraph, Presentation,
    Shape, Slide, Stroke, Table, TextFrame, TextRun,
};
use ppt_core::resolved::{
    ResolvedAnchor, ResolvedAutoShape, ResolvedBackground, ResolvedBodyProps, ResolvedBullet,
    ResolvedCell, ResolvedCellBorders, ResolvedConnector, ResolvedFill, ResolvedGroup,
    ResolvedParagraph, ResolvedPresentation, ResolvedRow, ResolvedRun, ResolvedShape,
    ResolvedSlide, ResolvedStroke, ResolvedTable, ResolvedTextFrame, DEFAULT_FONT_SIZE_PT,
    DEFAULT_INSET_LR_EMU, DEFAULT_INSET_TB_EMU,
};
use ppt_core::style::{
    Bullet, FontRef, PlaceholderRef, RunStyle, ShapeStyle, TextLevelStyle, TextStyleLevels,
    TxStyles,
};
use ppt_core::theme::{ClrMap, FontSet, Theme};

use crate::{InheritanceParts, ParsedPptx};

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
        shapes: slide
            .shapes
            .iter()
            .map(|sh| resolve_shape(sh, &ctx))
            .collect(),
    }
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
        Shape::Placeholder(gp) => ResolvedShape::Placeholder(gp.clone()),
    }
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
    }
}

fn resolve_table(t: &Table, ctx: &Ctx) -> ResolvedTable {
    // 单元格文字无占位符链;用非占位符基链(otherStyle + defaultTextStyle)。
    // `tableStyles.xml` 语义在 v1 之外(PRD §1)。
    let chain = style_chain(ctx, None, None, None, None);
    ResolvedTable {
        rect: t.rect,
        col_widths: t.col_widths.clone(),
        table_style_id: t.table_style_id.clone(),
        rows: t
            .rows
            .iter()
            .map(|row| ResolvedRow {
                cells: row
                    .cells
                    .iter()
                    .map(|c| resolve_cell(c, &chain, ctx))
                    .collect(),
                height: row.height,
            })
            .collect(),
    }
}

fn resolve_cell(cell: &Cell, chain: &[&TextStyleLevels], ctx: &Ctx) -> ResolvedCell {
    let border = |s: Option<&Stroke>| resolve_stroke(ctx, s, None);
    ResolvedCell {
        paragraphs: cell
            .paragraphs
            .iter()
            .map(|p| resolve_paragraph(p, chain, None, ctx))
            .collect(),
        col_span: cell.col_span,
        row_span: cell.row_span,
        fill: cell.fill.as_ref().map(|c| resolve_color(ctx, c, None)),
        merged: cell.merged,
        mar_l: cell.mar_l.unwrap_or(DEFAULT_INSET_LR_EMU),
        mar_r: cell.mar_r.unwrap_or(DEFAULT_INSET_LR_EMU),
        mar_t: cell.mar_t.unwrap_or(DEFAULT_INSET_TB_EMU),
        mar_b: cell.mar_b.unwrap_or(DEFAULT_INSET_TB_EMU),
        anchor: ResolvedAnchor::from_ooxml(cell.anchor.as_deref()),
        borders: ResolvedCellBorders {
            left: border(cell.borders.left.as_ref()),
            right: border(cell.borders.right.as_ref()),
            top: border(cell.borders.top.as_ref()),
            bottom: border(cell.borders.bottom.as_ref()),
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

/// 描边解析:显式 `a:ln` 字段逐项获胜;缺色 / 缺宽经 `lnRef` 从主题 `lnStyleLst` 补。
fn resolve_stroke(
    ctx: &Ctx,
    stroke: Option<&Stroke>,
    style: Option<&ShapeStyle>,
) -> Option<ResolvedStroke> {
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
    if color.is_none() && width_emu.is_none() && dash.is_none() {
        return None;
    }
    Some(ResolvedStroke {
        color,
        width_emu,
        dash,
    })
}

// ---- 颜色 / 字体 -----------------------------------------------------------

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
