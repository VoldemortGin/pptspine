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
//! (B-4)+ 组合仿射(B-5)+ 图表/SmartArt 占位框。**后续批次**:bodyPr 锚定/
//! 自适应(B-6)、表格(B-7)、背景(B-10)。

mod shapes;
mod text;
mod transform;

use std::collections::BTreeMap;
use std::path::Path;

use ppt_core::geom::emu_to_points;
use ppt_core::resolved::{ResolvedPresentation, ResolvedShape, ResolvedSlide};
use ppt_core::{PptError, Result};

pub use pdf_typeset::{ExportResult, ExportWarning};
use pdf_typeset::{Matrix, Op, PageOps, Typesetter};

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
    };
    let pages: Vec<PageOps> = pres
        .slides
        .iter()
        .map(|slide| PageOps {
            width,
            height,
            ops: slide_ops(&mut ts, &mut ctx, slide),
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
}

/// 一张 slide 的全部绘制 op(spTree 顺序 = 绘制顺序)。
fn slide_ops(ts: &mut Typesetter, ctx: &mut RenderCtx<'_>, slide: &ResolvedSlide) -> Vec<Op> {
    let mut ops = Vec::new();
    for shape in &slide.shapes {
        shape_ops(ts, ctx, shape, Flatten::IDENTITY, &mut ops);
    }
    ops
}

/// 文本体 → op(矩形经组合仿射映射;pptx `rot` 顺时针 → 引擎视觉逆时针取负;
/// 翻转不镜像文字,PowerPoint 语义)。
fn text_ops(
    ts: &mut Typesetter,
    rect: Option<ppt_core::geom::Rect>,
    tf: &ppt_core::resolved::ResolvedTextFrame,
    flat: Flatten,
    ops: &mut Vec<Op>,
) {
    let Some(rect) = rect else {
        return;
    };
    let spec = text::text_box_spec(flat.map_emu_rect(rect), -tf.xfrm.rot_deg(), &tf.paragraphs);
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
        ResolvedShape::TextBox(tf) => text_ops(ts, tf.rect, tf, flat, ops),
        ResolvedShape::Auto(auto) => {
            shapes::auto_shape_ops(ctx, auto, flat, ops);
            if let Some(tf) = &auto.text {
                text_ops(ts, tf.rect.or(auto.rect), tf, flat, ops);
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
        ResolvedShape::Placeholder(gp) => shapes::graphic_placeholder_ops(gp, flat, ops),
        // 表格的绝对单元格布局 / 边框 / 边距属 B-7,本批不绘制。
        ResolvedShape::Table(_) => {}
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
        }
    }

    fn para(text: &str) -> ResolvedParagraph {
        ResolvedParagraph {
            level: 0,
            align: None,
            mar_l: None,
            indent: None,
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
        pres(vec![ResolvedSlide { index: 0, shapes }])
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
            }),
        })
    }

    fn auto_shape(rect: Rect, geometry: &str, adjusts: Vec<(String, i64)>) -> ResolvedShape {
        ResolvedShape::Auto(ResolvedAutoShape {
            rect: Some(rect),
            xfrm: Xfrm::default(),
            geometry: Some(geometry.into()),
            adjusts,
            fill: Some(ResolvedFill::Solid(ResolvedColor::opaque([255, 0, 0]))),
            stroke: None,
            text: None,
        })
    }

    #[test]
    fn blank_slides_export_one_page_each() {
        let p = pres(vec![
            ResolvedSlide {
                index: 0,
                shapes: vec![],
            },
            ResolvedSlide {
                index: 1,
                shapes: vec![],
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
            stroke: None,
            text: None,
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
}
