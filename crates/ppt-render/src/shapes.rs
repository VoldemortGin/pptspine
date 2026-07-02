//! 形状底绘制(B-1 路径):预设几何轮廓的 fill/stroke、连接线、图片放置、
//! 图表/SmartArt 占位框。
//!
//! 轮廓来自 `pdf_typeset::preset::preset_outline`(TS-6);预设名在 v1 子集外时
//! 得到包围盒退化 + [`ExportWarning::PresetDegraded`]。`avLst` 调整值与
//! `rot`/`flipH`/`flipV` 属 B-4(尚未进 IR),此处传空 / 不变换。

use ppt_core::color::ResolvedColor;
use ppt_core::geom::{emu_to_points, Rect as EmuRect};
use ppt_core::model::{GraphicPlaceholder, Picture};
use ppt_core::resolved::{ResolvedAutoShape, ResolvedConnector, ResolvedStroke};

use pdf_typeset::preset::preset_outline;
use pdf_typeset::{ExportWarning, Fill, Op, Rect, Rgb, Stroke, Typesetter};

use crate::text::rgb;
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

/// EMU 矩形 → 引擎页坐标矩形(pt)。
fn pt_rect(r: EmuRect) -> Rect {
    let (x, y, w, h) = r.to_points();
    Rect::new(x, y, x + w, y + h)
}

/// 终端色 → 引擎填充(带常数 alpha)。
fn fill_of(c: ResolvedColor) -> Fill {
    Fill {
        color: rgb(c),
        alpha: c.alpha.map_or(1.0, f64::from),
        even_odd: false,
    }
}

/// 已解析描边 → 引擎描边(颜色缺省黑;线宽缺省 0.75 pt;虚线映射属 B-4,先实线)。
fn stroke_of(s: &ResolvedStroke) -> Stroke {
    let color = s.color.unwrap_or(ResolvedColor::opaque([0, 0, 0]));
    let mut stroke = Stroke::new(
        rgb(color),
        s.width_emu.map_or(DEFAULT_STROKE_PT, emu_to_points),
    );
    stroke.alpha = color.alpha.map_or(1.0, f64::from);
    stroke
}

/// 预设几何轮廓 → `Op::Path`(fill 与 stroke 皆缺时不发 op);
/// 子集外预设记一次 [`ExportWarning::PresetDegraded`]。
fn outline_op(
    ctx: &mut RenderCtx<'_>,
    geometry: Option<&str>,
    default_geometry: &str,
    rect: EmuRect,
    fill: Option<Fill>,
    stroke: Option<Stroke>,
    ops: &mut Vec<Op>,
) {
    if fill.is_none() && stroke.is_none() {
        return;
    }
    let name = geometry.unwrap_or(default_geometry);
    let outline = preset_outline(name, pt_rect(rect), &[]);
    if outline.degraded {
        ctx.warnings.push(ExportWarning::PresetDegraded {
            preset: name.to_string(),
        });
    }
    ops.push(Op::Path {
        segs: outline.segs,
        fill,
        stroke,
    });
}

/// 自选图形的形状底(文字由调用方在其上叠加)。
pub(crate) fn auto_shape_ops(ctx: &mut RenderCtx<'_>, auto: &ResolvedAutoShape, ops: &mut Vec<Op>) {
    let Some(rect) = auto.rect else {
        return;
    };
    outline_op(
        ctx,
        auto.geometry.as_deref(),
        "rect",
        rect,
        auto.fill.map(fill_of),
        auto.stroke.as_ref().map(stroke_of),
        ops,
    );
}

/// 连接线:仅描边(无描边解析结果时以缺省黑线兜底,保证可见)。
pub(crate) fn connector_ops(ctx: &mut RenderCtx<'_>, conn: &ResolvedConnector, ops: &mut Vec<Op>) {
    let Some(rect) = conn.rect else {
        return;
    };
    let stroke = conn
        .stroke
        .as_ref()
        .map_or_else(|| Stroke::new(Rgb::BLACK, DEFAULT_STROKE_PT), stroke_of);
    outline_op(
        ctx,
        conn.geometry.as_deref(),
        "line",
        rect,
        conn.fill.map(fill_of),
        Some(stroke),
        ops,
    );
}

/// 图片放置:embed 一次(按 media 裸名 / rel id 缓存 id),多次放置复用同一 id。
pub(crate) fn picture_ops(
    ts: &mut Typesetter,
    ctx: &mut RenderCtx<'_>,
    pic: &Picture,
    ops: &mut Vec<Op>,
) {
    let Some(rect) = pic.rect else {
        return;
    };
    let key = pic.media_name.clone().unwrap_or_else(|| pic.rel_id.clone());
    let id = if let Some(&cached) = ctx.image_ids.get(&key) {
        cached
    } else {
        let id = match pic.media_name.as_deref().and_then(|n| ctx.media.get(n)) {
            Some(bytes) => {
                let (_, _, w, h) = rect.to_points();
                ts.add_image(&pdf_typeset::ImageSpec::new(bytes.clone(), w, h))
            }
            None => {
                ctx.warnings.push(ExportWarning::ImageDropped {
                    reason: format!("media '{key}' not found in package"),
                });
                None
            }
        };
        ctx.image_ids.insert(key, id);
        id
    };
    if let Some(id) = id {
        let (x, y, w, h) = rect.to_points();
        ops.push(Op::Image { id, x, y, w, h });
    }
}

/// 图表 / SmartArt / OLE:浅灰占位框(PRD §1 v1 降级;专属告警种类待引擎侧扩充)。
pub(crate) fn graphic_placeholder_ops(gp: &GraphicPlaceholder, ops: &mut Vec<Op>) {
    let Some(rect) = gp.rect else {
        return;
    };
    let r = pt_rect(rect);
    ops.push(Op::Path {
        segs: vec![
            pdf_typeset::PathSeg::MoveTo { x: r.x0, y: r.y0 },
            pdf_typeset::PathSeg::LineTo { x: r.x1, y: r.y0 },
            pdf_typeset::PathSeg::LineTo { x: r.x1, y: r.y1 },
            pdf_typeset::PathSeg::LineTo { x: r.x0, y: r.y1 },
            pdf_typeset::PathSeg::Close,
        ],
        fill: Some(Fill::new(PLACEHOLDER_FILL)),
        stroke: Some(Stroke::new(PLACEHOLDER_STROKE, DEFAULT_STROKE_PT)),
    });
}
