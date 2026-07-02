#![forbid(unsafe_code)]
//! `ppt-render` —— 把继承链解析后的终态 IR([`ppt_core::resolved`])映射到共享
//! 排版引擎 `pdf-typeset`(pdfspine Phase A),产出**逐 slide 一页、绝对定位**的
//! 忠实 PDF(PRD-PDF-EXPORT §5,B-1/B-2)。
//!
//! 职责边界:本 crate 只做 IR → 引擎输入的**薄映射**;所有字体解析 / 布局 / PDF
//! 序列化都在 `pdf-typeset` 内。坐标契约:一切 op 以**左上原点、y 向下**的页坐标
//! 表达,发射时由引擎统一翻转(`pdf_typeset::ops` 的约定)。
//!
//! 本批(B-1/B-2)覆盖:空白页装配 + 显式几何文本框 + 预设形状底(fill/stroke)+
//! 图片放置 + 图表/SmartArt 占位框。**后续批次**:xfrm rot/flip 与 avLst(B-4)、
//! 组合仿射(B-5)、bodyPr 锚定/自适应(B-6)、表格(B-7)、背景(B-10)。

mod shapes;
mod text;

use std::collections::BTreeMap;
use std::path::Path;

use ppt_core::geom::emu_to_points;
use ppt_core::resolved::{ResolvedPresentation, ResolvedShape, ResolvedSlide};
use ppt_core::{PptError, Result};

pub use pdf_typeset::{ExportResult, ExportWarning};
use pdf_typeset::{Op, PageOps, Typesetter};

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
        shape_ops(ts, ctx, shape, &mut ops);
    }
    ops
}

/// 单个形状 → op 序列(组合递归;B-5 前子形状保持原始子坐标)。
fn shape_ops(
    ts: &mut Typesetter,
    ctx: &mut RenderCtx<'_>,
    shape: &ResolvedShape,
    ops: &mut Vec<Op>,
) {
    match shape {
        ResolvedShape::TextBox(tf) => {
            if let Some(spec) = text::text_box_spec(tf.rect, &tf.paragraphs) {
                ops.extend(ts.layout_text_box(&spec));
            }
        }
        ResolvedShape::Auto(auto) => {
            shapes::auto_shape_ops(ctx, auto, ops);
            if let Some(tf) = &auto.text {
                let rect = tf.rect.or(auto.rect);
                if let Some(spec) = text::text_box_spec(rect, &tf.paragraphs) {
                    ops.extend(ts.layout_text_box(&spec));
                }
            }
        }
        ResolvedShape::Connector(conn) => shapes::connector_ops(ctx, conn, ops),
        ResolvedShape::Picture(pic) => shapes::picture_ops(ts, ctx, pic, ops),
        ResolvedShape::Group(children) => {
            // B-5(chOff/chExt 仿射重映射)落地前按原样递归:子形状以原始子坐标绘制。
            for child in children {
                shape_ops(ts, ctx, child, ops);
            }
        }
        ResolvedShape::Placeholder(gp) => shapes::graphic_placeholder_ops(gp, ops),
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
    use ppt_core::model::RunKind;
    use ppt_core::resolved::{
        ResolvedAutoShape, ResolvedBullet, ResolvedParagraph, ResolvedRun, ResolvedTextFrame,
    };

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

    fn pres(slides: Vec<ResolvedSlide>) -> ResolvedPresentation {
        ResolvedPresentation {
            slide_size: (12_192_000, 6_858_000), // 16:9 => 960x540 pt
            slides,
        }
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
        let out = render_pdf(&p, &BTreeMap::new(), &RenderOptions::default()).expect("render");
        assert!(out.pdf.starts_with(b"%PDF-"));
        // 两页:页面树 /Count 2。
        let hay = String::from_utf8_lossy(&out.pdf);
        assert!(hay.contains("/Count 2"), "expected 2-page tree");
    }

    #[test]
    fn text_box_embeds_exactly_one_face() {
        let slide = ResolvedSlide {
            index: 0,
            shapes: vec![ResolvedShape::TextBox(ResolvedTextFrame {
                rect: Some(Rect::new(914_400, 914_400, 4_572_000, 1_828_800)),
                paragraphs: vec![para("Hello render")],
            })],
        };
        let out = render_pdf(
            &pres(vec![slide]),
            &BTreeMap::new(),
            &RenderOptions::default(),
        )
        .expect("render");
        let count = out
            .pdf
            .windows(b"/FontFile2".len())
            .filter(|w| w == b"/FontFile2")
            .count();
        assert_eq!(count, 1, "exactly one FontFile2 per used face");
    }

    #[test]
    fn unknown_preset_degrades_with_warning() {
        let slide = ResolvedSlide {
            index: 0,
            shapes: vec![ResolvedShape::Auto(ResolvedAutoShape {
                rect: Some(Rect::new(0, 0, 914_400, 914_400)),
                geometry: Some("cloud".into()),
                fill: Some(ResolvedColor::opaque([255, 0, 0])),
                stroke: None,
                text: None,
            })],
        };
        let out = render_pdf(
            &pres(vec![slide]),
            &BTreeMap::new(),
            &RenderOptions::default(),
        )
        .expect("render");
        assert!(out
            .warnings
            .iter()
            .any(|w| matches!(w, ExportWarning::PresetDegraded { preset } if preset == "cloud")));
    }

    #[test]
    fn missing_media_reports_image_dropped() {
        let slide = ResolvedSlide {
            index: 0,
            shapes: vec![ResolvedShape::Picture(ppt_core::model::Picture {
                rect: Some(Rect::new(0, 0, 914_400, 914_400)),
                rel_id: "rId1".into(),
                media_name: Some("missing.png".into()),
                image_bytes_len: 0,
                placeholder: None,
            })],
        };
        let out = render_pdf(
            &pres(vec![slide]),
            &BTreeMap::new(),
            &RenderOptions::default(),
        )
        .expect("render");
        assert!(out
            .warnings
            .iter()
            .any(|w| matches!(w, ExportWarning::ImageDropped { .. })));
    }
}
