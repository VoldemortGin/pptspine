//! 形状底绘制(B-1/B-4 路径):预设几何轮廓的 fill/stroke(含 `avLst` 调整值
//! 透传与 `prstDash` 虚线)、形状级 rot/flip 仿射、连接线、图片放置(含
//! `srcRect` 裁剪 / `fillRect` 拉伸目标)、图表/SmartArt 占位框。
//!
//! 轮廓来自 `pdf_typeset::preset::preset_outline`(TS-6);预设名在 v1 子集外时
//! 得到包围盒退化 + [`ExportWarning::PresetDegraded`]。所有矩形先经组合仿射
//! [`Flatten`](B-5)映射到页坐标,再做形状级变换(绕映射后矩形中心,均匀缩放
//! 与旋转/翻转可交换,次序安全)。
//!
//! `srcRect` 裁剪(§3.n)走**消费侧几何**实现:引擎 `ImageSpec` 尚无裁剪参数
//! (本批引擎 pin 不动),把整图按 `显示宽 / (1 − l − r)` 放大、以负偏移铺放,
//! 再用显示矩形做 `Op::Group { clip }` 剪裁——无需解码重编码,对 JPEG 直通路径
//! 零损;负值(外扩)同式自然成立。
//!
//! 线端装饰(`a:headEnd` / `a:tailEnd`)见 [`line_ends`]:只加在开放轮廓(line /
//! 连接线 / arc)两端,随形状级变换一起生效。`a:custGeom` v1 不求值路径公式:自选
//! 图形按包围盒矩形、连接线按缺省直线降级,画了东西就记 `custom-geometry-approximated`。

mod line_ends;

use ppt_core::color::ResolvedColor;
use ppt_core::geom::emu_to_points;
use ppt_core::model::{BlipFill, GraphicPlaceholder, LineEnd, Picture, RelRect, Xfrm};
use ppt_core::resolved::{ResolvedAutoShape, ResolvedConnector, ResolvedFill, ResolvedStroke};

use pdf_typeset::preset::preset_outline;
use pdf_typeset::{ExportWarning, Fill, Op, PathSeg, Rect, Rgb, Stroke, Typesetter};

use crate::text::rgb;
use crate::transform::{shape_transform, Flatten};
use crate::RenderCtx;

/// OOXML `a:ln@w` 缺省线宽(9525 EMU = 0.75 pt)。
const DEFAULT_STROKE_PT: f64 = 0.75;
/// 图表 / SmartArt / OLE 占位框的填充灰。
const PLACEHOLDER_FILL: Rgb = Rgb {
    r: 0.85,
    g: 0.85,
    b: 0.85,
};
/// 占位框描边灰。
const PLACEHOLDER_STROKE: Rgb = Rgb {
    r: 0.6,
    g: 0.6,
    b: 0.6,
};

/// 终态填充 → 引擎填充(带常数 alpha);渐变降级记 [`ExportWarning::GradientDegraded`]。
fn fill_of(ctx: &mut RenderCtx<'_>, f: ResolvedFill) -> Fill {
    match f {
        ResolvedFill::Gradient(_) => ctx.warnings.push(ExportWarning::GradientDegraded {
            kind: "gradFill".to_string(),
        }),
        ResolvedFill::Pattern(_) => ctx.warnings.push(ExportWarning::Custom {
            kind: PATTERN_FILL_KIND.to_string(),
            detail: "图案填充(pattFill)v1 降级为前景 / 背景平均色的纯色".to_string(),
        }),
        ResolvedFill::Solid(_) => {}
    }
    let c = f.color();
    Fill {
        color: rgb(c),
        alpha: c.alpha.map_or(1.0, f64::from),
        even_odd: false,
    }
}

/// 已解析描边 → 引擎描边(颜色缺省黑;线宽缺省 0.75 pt,随组合缩放;
/// `prstDash` 按 DrawingML 语义折成以线宽为单位的 dash 数组)。
fn stroke_of(s: &ResolvedStroke, scale: f64) -> Stroke {
    let color = s.color.unwrap_or(ResolvedColor::opaque([0, 0, 0]));
    let width = s.width_emu.map_or(DEFAULT_STROKE_PT, emu_to_points) * scale;
    let mut stroke = Stroke::new(rgb(color), width);
    stroke.alpha = color.alpha.map_or(1.0, f64::from);
    stroke.dashes = dash_pattern(s.dash.as_deref(), width);
    stroke
}

/// `a:prstDash@val` → PDF dash 数组(pt)。DrawingML 预设以**线宽**为单位
/// (ECMA-376 §20.1.10.48 的通行实现值);`solid` / 未知 → 空数组(实线)。
fn dash_pattern(dash: Option<&str>, width: f64) -> Vec<f64> {
    let units: &[f64] = match dash {
        Some("dash") => &[4.0, 3.0],
        Some("dashDot") => &[4.0, 3.0, 1.0, 3.0],
        Some("dot") => &[1.0, 3.0],
        Some("lgDash") => &[8.0, 3.0],
        Some("lgDashDot") => &[8.0, 3.0, 1.0, 3.0],
        Some("lgDashDotDot") => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
        Some("sysDash") => &[3.0, 1.0],
        Some("sysDot") => &[1.0, 1.0],
        Some("sysDashDot") => &[3.0, 1.0, 1.0, 1.0],
        Some("sysDashDotDot") => &[3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        _ => &[],
    };
    units.iter().map(|u| u * width).collect()
}

/// 矩形 → 闭合路径段(clip / 占位框共用)。
fn rect_segs(r: Rect) -> Vec<PathSeg> {
    vec![
        PathSeg::MoveTo { x: r.x0, y: r.y0 },
        PathSeg::LineTo { x: r.x1, y: r.y0 },
        PathSeg::LineTo { x: r.x1, y: r.y1 },
        PathSeg::LineTo { x: r.x0, y: r.y1 },
        PathSeg::Close,
    ]
}

/// 降级告警种类:图案填充按两色平均色的纯色画。
const PATTERN_FILL_KIND: &str = "pattern-fill-degraded";
/// 降级告警种类:图片填充的 `a:tile` 平铺按拉伸画。
const BLIP_TILED_KIND: &str = "blip-fill-tiled";
/// 降级告警种类:`a:custGeom` 按包围盒 / 缺省直线近似。
const CUSTOM_GEOMETRY_KIND: &str = "custom-geometry-approximated";
/// 降级告警种类:线端装饰画不出(规范外 type / 轮廓降级后无开放端点)。
const LINE_END_KIND: &str = "line-end-degraded";

/// 一条描边的两端装饰(已解析;`None` / `type="none"` = 不画)。
#[derive(Clone, Copy, Default)]
struct LineEnds<'a> {
    head: Option<&'a LineEnd>,
    tail: Option<&'a LineEnd>,
}

impl<'a> LineEnds<'a> {
    fn of(s: Option<&'a ResolvedStroke>) -> Self {
        LineEnds {
            head: s.and_then(|s| s.head_end.as_ref()),
            tail: s.and_then(|s| s.tail_end.as_ref()),
        }
    }

    fn any(self) -> bool {
        line_ends::is_live(self.head) || line_ends::is_live(self.tail)
    }
}

/// `a:custGeom` 近似绘制的告警(只在确实画了东西时发)。
fn custom_geometry_warning(ctx: &mut RenderCtx<'_>, drawn_as: &str) {
    ctx.warnings.push(ExportWarning::Custom {
        kind: CUSTOM_GEOMETRY_KIND.to_string(),
        detail: format!("custGeom 自定义路径 v1 未求值;按{drawn_as}近似绘制"),
    });
}

/// 把一个 op 按形状级 rot/flip 包进 `Op::Group`(恒等直接透传)。
fn with_shape_transform(xfrm: Xfrm, rect: Rect, op: Op, ops: &mut Vec<Op>) {
    match shape_transform(xfrm, rect) {
        Some(m) => ops.push(Op::Group {
            transform: Some(m),
            clip: None,
            ops: vec![op],
        }),
        None => ops.push(op),
    }
}

/// 预设几何轮廓 → `Op::Path`(fill 与 stroke 皆缺时不发 op);`avLst` 调整值
/// 原样透传 TS-6;子集外预设记一次 [`ExportWarning::PresetDegraded`];
/// rot/flip 绕映射后矩形中心生效。有线端装饰时线身按需缩短、装饰 op 紧随其后,
/// 与线身同组变换;`custom` = 几何来自 `a:custGeom`(按缺省几何近似 + 告警)。
#[allow(clippy::too_many_arguments)]
fn outline_op(
    ctx: &mut RenderCtx<'_>,
    geometry: Option<&str>,
    default_geometry: &str,
    rect: Rect,
    xfrm: Xfrm,
    adjusts: &[(String, i64)],
    fill: Option<Fill>,
    stroke: Option<Stroke>,
    ends: LineEnds<'_>,
    custom: bool,
    scale: f64,
    ops: &mut Vec<Op>,
) {
    if fill.is_none() && stroke.is_none() {
        return;
    }
    if custom {
        let drawn_as = if default_geometry == "line" {
            "缺省直线"
        } else {
            "包围盒矩形"
        };
        custom_geometry_warning(ctx, drawn_as);
    }
    let name = geometry.unwrap_or(default_geometry);
    #[allow(clippy::cast_precision_loss)]
    let adj: Vec<(&str, f64)> = adjusts
        .iter()
        .map(|(n, v)| (n.as_str(), *v as f64))
        .collect();
    let outline = preset_outline(name, rect, &adj);
    if outline.degraded {
        ctx.warnings.push(ExportWarning::PresetDegraded {
            preset: name.to_string(),
        });
    }
    let line = stroke.as_ref().filter(|_| ends.any()).cloned();
    let Some(line) = line else {
        let path = Op::Path {
            segs: outline.segs,
            fill,
            stroke,
        };
        with_shape_transform(xfrm, rect, path, ops);
        return;
    };
    let mut segs = outline.segs;
    let min_base = line_ends::MIN_BASE_PT * scale;
    let decorations = line_ends::decorate(&mut segs, ends.head, ends.tail, &line, min_base);
    let mut group = vec![Op::Path { segs, fill, stroke }];
    match decorations {
        Some((deco, skipped)) => {
            group.extend(deco);
            for line_ends::EndSkip::UnknownKind(kind) in skipped {
                ctx.warnings.push(ExportWarning::Custom {
                    kind: LINE_END_KIND.to_string(),
                    detail: format!("线端装饰 type='{kind}' 未支持;该端不画"),
                });
            }
        }
        // 闭合轮廓本就不带线端(PowerPoint 同);只有降级 / 近似成闭合时才算丢失。
        None if outline.degraded || custom => ctx.warnings.push(ExportWarning::Custom {
            kind: LINE_END_KIND.to_string(),
            detail: format!("'{name}' 轮廓已降级为闭合包围盒,线端装饰无处可放;不画"),
        }),
        None => {}
    }
    match shape_transform(xfrm, rect) {
        Some(m) => ops.push(Op::Group {
            transform: Some(m),
            clip: None,
            ops: group,
        }),
        None => ops.extend(group),
    }
}

/// 自选图形的形状底(文字由调用方在其上叠加)。
pub(crate) fn auto_shape_ops(
    ts: &mut Typesetter,
    ctx: &mut RenderCtx<'_>,
    auto: &ResolvedAutoShape,
    flat: Flatten,
    ops: &mut Vec<Op>,
) {
    let Some(rect) = auto.rect else {
        return;
    };
    if let Some(b) = &auto.blip_fill {
        blip_fill_ops(ts, ctx, auto, b, flat.map_emu_rect(rect), ops);
    }
    let fill = auto.fill.map(|f| fill_of(ctx, f));
    let stroke = auto.stroke.as_ref().map(|s| stroke_of(s, flat.s));
    outline_op(
        ctx,
        auto.geometry.as_deref(),
        "rect",
        flat.map_emu_rect(rect),
        auto.xfrm,
        &auto.adjusts,
        fill,
        stroke,
        LineEnds::of(auto.stroke.as_ref()),
        auto.custom_geometry,
        flat.s,
        ops,
    );
}

/// 形状的图片填充:图片按 `fillRect` / `srcRect` 放置(与 `p:pic` 同一套 [`placed_image_op`]、
/// 同一份 embed 缓存),再以**形状几何轮廓**为剪裁路径(引擎 `Op::Group.clip` 接受任意路径),
/// 铺在描边 / 文字之下。`a:tile` 平铺 v1 按拉伸画并告警;图片缺失记 `ImageDropped`。
fn blip_fill_ops(
    ts: &mut Typesetter,
    ctx: &mut RenderCtx<'_>,
    auto: &ResolvedAutoShape,
    b: &BlipFill,
    r: Rect,
    ops: &mut Vec<Op>,
) {
    let key = b.media_name.clone().unwrap_or_else(|| b.rel_id.clone());
    let Some(id) = embed_image(ts, ctx, b.media_name.as_deref(), &key, r) else {
        return;
    };
    if b.tile {
        ctx.warnings.push(ExportWarning::Custom {
            kind: BLIP_TILED_KIND.to_string(),
            detail: "图片填充 a:tile 平铺 v1 按拉伸画".to_string(),
        });
    }
    let name = auto.geometry.as_deref().unwrap_or("rect");
    #[allow(clippy::cast_precision_loss)]
    let adj: Vec<(&str, f64)> = auto
        .adjusts
        .iter()
        .map(|(n, v)| (n.as_str(), *v as f64))
        .collect();
    let outline = preset_outline(name, r, &adj);
    // 无描边时 `outline_op` 不会跑(fill 为空),几何降级的告警在此补发,避免静默。
    if auto.stroke.is_none() {
        if outline.degraded {
            ctx.warnings.push(ExportWarning::PresetDegraded {
                preset: name.to_string(),
            });
        }
        if auto.custom_geometry {
            custom_geometry_warning(ctx, "包围盒矩形");
        }
    }
    let image = placed_image_op(id, r, b.fill_rect, b.src_rect);
    let clipped = Op::Group {
        transform: None,
        clip: Some(outline.segs),
        ops: vec![image],
    };
    with_shape_transform(auto.xfrm, r, clipped, ops);
}

/// 连接线:仅描边(无描边解析结果时以缺省黑线兜底,保证可见);
/// 方向常靠 flipH/flipV 表达,经形状级变换生效。
pub(crate) fn connector_ops(
    ctx: &mut RenderCtx<'_>,
    conn: &ResolvedConnector,
    flat: Flatten,
    ops: &mut Vec<Op>,
) {
    let Some(rect) = conn.rect else {
        return;
    };
    let fill = conn.fill.map(|f| fill_of(ctx, f));
    let stroke = if conn.no_line {
        None
    } else {
        Some(conn.stroke.as_ref().map_or_else(
            || Stroke::new(Rgb::BLACK, DEFAULT_STROKE_PT * flat.s),
            |s| stroke_of(s, flat.s),
        ))
    };
    outline_op(
        ctx,
        conn.geometry.as_deref(),
        "line",
        flat.map_emu_rect(rect),
        conn.xfrm,
        &conn.adjusts,
        fill,
        stroke,
        LineEnds::of(conn.stroke.as_ref()),
        conn.custom_geometry,
        flat.s,
        ops,
    );
}

/// `a:srcRect` / `a:fillRect` 千分之一百分点 → 分数。
fn frac(v: i32) -> f64 {
    f64::from(v) / 100_000.0
}

/// `a:stretch > a:fillRect`:显示矩形按分数内收(负值外扩)。
fn apply_fill_rect(r: Rect, fr: RelRect) -> Rect {
    let (w, h) = (r.x1 - r.x0, r.y1 - r.y0);
    Rect::new(
        r.x0 + frac(fr.l) * w,
        r.y0 + frac(fr.t) * h,
        r.x1 - frac(fr.r) * w,
        r.y1 - frac(fr.b) * h,
    )
}

/// `a:srcRect` 裁剪 → 整图的铺放矩形:被裁剩的源分数拉伸填满 `dest`,故整图
/// 放大 `1/(1−l−r)` 倍、负偏移对齐;调用方以 `dest` 剪裁。裁剩分数退化
/// (≤ 0)时放弃裁剪,返回 `None`。
fn crop_placement(dest: Rect, sr: RelRect) -> Option<Rect> {
    if sr == RelRect::default() {
        return None;
    }
    let (rw, rh) = (1.0 - frac(sr.l) - frac(sr.r), 1.0 - frac(sr.t) - frac(sr.b));
    if rw <= f64::EPSILON || rh <= f64::EPSILON {
        return None;
    }
    let w = (dest.x1 - dest.x0) / rw;
    let h = (dest.y1 - dest.y0) / rh;
    let x = dest.x0 - frac(sr.l) * w;
    let y = dest.y0 - frac(sr.t) * h;
    Some(Rect::new(x, y, x + w, y + h))
}

/// 把一张图片 embed 进引擎(按 `key` 缓存 id:同图多次放置只 embed 一份);media 缺失记
/// `ImageDropped` 并缓存失败结果。`size` 仅作 `ImageSpec` 的初始显示尺寸。
fn embed_image(
    ts: &mut Typesetter,
    ctx: &mut RenderCtx<'_>,
    media_name: Option<&str>,
    key: &str,
    size: Rect,
) -> Option<usize> {
    if let Some(&cached) = ctx.image_ids.get(key) {
        return cached;
    }
    let id = match media_name.and_then(|n| ctx.media.get(n)) {
        Some(bytes) => ts.add_image(&pdf_typeset::ImageSpec::new(
            bytes.clone(),
            size.x1 - size.x0,
            size.y1 - size.y0,
        )),
        None => {
            ctx.warnings.push(ExportWarning::ImageDropped {
                reason: format!("media '{key}' not found in package"),
            });
            None
        }
    };
    ctx.image_ids.insert(key.to_string(), id);
    id
}

/// 图片在 `r` 内的放置 op:`fillRect` 定显示矩形,`srcRect` 以整图放大 + 显示矩形剪裁
/// 实现源裁剪(图片 `p:pic` 与形状图片填充共用)。
fn placed_image_op(
    id: usize,
    r: Rect,
    fill_rect: Option<RelRect>,
    src_rect: Option<RelRect>,
) -> Op {
    let dest = fill_rect.map_or(r, |fr| apply_fill_rect(r, fr));
    let image_at = |p: Rect| Op::Image {
        id,
        x: p.x0,
        y: p.y0,
        w: p.x1 - p.x0,
        h: p.y1 - p.y0,
    };
    match src_rect.and_then(|sr| crop_placement(dest, sr)) {
        Some(full) => Op::Group {
            transform: None,
            clip: Some(rect_segs(dest)),
            ops: vec![image_at(full)],
        },
        None => image_at(dest),
    }
}

/// 图片放置:embed 一次(按 media 裸名 / rel id 缓存 id),多次放置复用同一 id;
/// `fillRect` 定显示矩形、`srcRect` 以放大 + 剪裁实现源裁剪、rot/flip 绕形状
/// 矩形中心生效(翻转对图片是真镜像)。
pub(crate) fn picture_ops(
    ts: &mut Typesetter,
    ctx: &mut RenderCtx<'_>,
    pic: &Picture,
    flat: Flatten,
    ops: &mut Vec<Op>,
) {
    let Some(rect) = pic.rect else {
        return;
    };
    let r = flat.map_emu_rect(rect);
    let key = pic.media_name.clone().unwrap_or_else(|| pic.rel_id.clone());
    let Some(id) = embed_image(ts, ctx, pic.media_name.as_deref(), &key, r) else {
        return;
    };
    let base = placed_image_op(id, r, pic.fill_rect, pic.src_rect);
    with_shape_transform(pic.xfrm, r, base, ops);
}

/// SmartArt 降级告警的 `Custom` 种类标签。
const SMARTART_DEGRADED_KIND: &str = "smartart-degraded";

/// 图表:有缓存数据且类型受支持时画矢量图表([`crate::chart`]);否则(含图表部件缺失)
/// 记 `chart-degraded` 告警并画浅灰占位框。SmartArt(无 drawing 部件才会走到这里)记
/// `smartart-degraded` 并画占位框;OLE:浅灰占位框(PRD §1 v1 降级)。
pub(crate) fn graphic_placeholder_ops(
    ts: &mut Typesetter,
    ctx: &mut RenderCtx<'_>,
    gp: &GraphicPlaceholder,
    flat: Flatten,
    ops: &mut Vec<Op>,
) {
    let Some(rect) = gp.rect else {
        return;
    };
    let frame = flat.map_emu_rect(rect);
    match &gp.chart {
        Some(chart) => {
            if crate::chart::chart_ops(ts, ctx, chart, frame, ops) {
                return;
            }
        }
        None if gp.kind.as_deref().is_some_and(|k| k.ends_with("/chart")) => {
            ctx.warnings.push(ExportWarning::Custom {
                kind: crate::chart::CHART_DEGRADED_KIND.to_string(),
                detail: "图表部件缺失或无法读取;画占位框".to_string(),
            });
        }
        None if gp.kind.as_deref().is_some_and(|k| k.ends_with("/diagram")) => {
            ctx.warnings.push(ExportWarning::Custom {
                kind: SMARTART_DEGRADED_KIND.to_string(),
                detail: "SmartArt 无可用 drawing 部件;画占位框(data 部件文字仅进文本导出)"
                    .to_string(),
            });
        }
        None => {}
    }
    ops.push(Op::Path {
        segs: rect_segs(frame),
        fill: Some(Fill::new(PLACEHOLDER_FILL)),
        stroke: Some(Stroke::new(PLACEHOLDER_STROKE, DEFAULT_STROKE_PT)),
    });
}
