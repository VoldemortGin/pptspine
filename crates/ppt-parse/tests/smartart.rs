//! SmartArt(`dgm:relIds`)验收:优先读 drawing 部件(`dsp:spTree` 形状 → frame 变换后的
//! 组合形状),退回 data 部件(`dgm:pt > dgm:t` 文字),两者皆缺 / 畸形保持占位框;
//! 文字进入 `to_text` / Markdown。pptx 现场合成,不落二进制 fixture。

use std::io::{Cursor, Write};

use ppt_core::export::{presentation_markdown_with, presentation_text_with, ExportOptions};
use ppt_core::geom::Rect;
use ppt_core::model::Shape;
use ppt_core::DiagnosticKind;
use ppt_parse::{parse_bytes, parse_bytes_with_limits, resolve, ZipLimits};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const DGM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
const REL_DATA: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData";
const REL_DRAWING: &str = "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing";

const FRAME: Rect = Rect::new(1_000_000, 2_000_000, 4_000_000, 2_000_000);

fn frame(dm: &str) -> String {
    let relids = if dm.is_empty() {
        String::new()
    } else {
        format!(
            r#"<dgm:relIds xmlns:dgm="{DGM}" r:dm="{dm}" r:lo="rId3" r:qs="rId4" r:cs="rId5"/>"#
        )
    };
    format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Diagram"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
  <p:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></p:xfrm>
  <a:graphic><a:graphicData uri="{DGM}">{relids}</a:graphicData></a:graphic></p:graphicFrame>"#,
        FRAME.x, FRAME.y, FRAME.w, FRAME.h
    )
}

fn pt(attrs: &str, paras: &[&str]) -> String {
    let body: String = paras
        .iter()
        .map(|t| {
            if t.is_empty() {
                "<a:p><a:endParaRPr lang=\"en-US\"/></a:p>".to_string()
            } else {
                format!("<a:p><a:r><a:rPr lang=\"en-US\"/><a:t>{t}</a:t></a:r></a:p>")
            }
        })
        .collect();
    format!(
        r#"<dgm:pt modelId="{{X}}" {attrs}><dgm:prSet/><dgm:spPr/><dgm:t><a:bodyPr/><a:lstStyle/>{body}</dgm:t></dgm:pt>"#
    )
}

/// data 部件:doc 根点(无字)+ 两个内容点 + 三种非内容点(带"哨兵"文字,必须被跳过)。
fn data_xml(drawing_rel: Option<&str>) -> String {
    let ext = drawing_rel
        .map(|id| {
            format!(
                r#"<dgm:extLst><a:ext uri="http://schemas.microsoft.com/office/drawing/2008/diagram">
  <dsp:dataModelExt xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" relId="{id}" minVer="{DGM}"/></a:ext></dgm:extLst>"#
            )
        })
        .unwrap_or_default();
    let pts = [
        pt(r#"type="doc""#, &[""]),
        pt("", &["Alpha"]),
        pt(r#"type="parTrans""#, &["SENTINEL-PAR"]),
        pt(r#"type="pres""#, &["SENTINEL-PRES"]),
        pt("", &["Beta", "", "Gamma"]),
        pt(r#"type="sibTrans""#, &["SENTINEL-SIB"]),
        pt(r#"type="asst""#, &["Delta"]),
    ]
    .concat();
    format!(
        r#"<dgm:dataModel xmlns:dgm="{DGM}" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
<dgm:ptLst>{pts}</dgm:ptLst><dgm:cxnLst/>{ext}</dgm:dataModel>"#
    )
}

fn dsp_sp(x: i64, y: i64, w: i64, h: i64, text: &str) -> String {
    format!(
        r#"<dsp:sp modelId="{{S}}"><dsp:nvSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvSpPr/></dsp:nvSpPr>
<dsp:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></a:xfrm>
<a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></dsp:spPr>
<dsp:style><a:lnRef idx="2"><a:scrgbClr r="0" g="0" b="0"/></a:lnRef><a:fillRef idx="1"><a:scrgbClr r="0" g="0" b="0"/></a:fillRef>
<a:effectRef idx="0"><a:scrgbClr r="0" g="0" b="0"/></a:effectRef><a:fontRef idx="minor"/></dsp:style>
<dsp:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US"/><a:t>{text}</a:t></a:r></a:p></dsp:txBody>
<dsp:txXfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></dsp:txXfrm></dsp:sp>"#
    )
}

fn drawing_xml(inner: &str) -> String {
    format!(
        r#"<dsp:drawing xmlns:dgm="{DGM}" xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram"
 xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dsp:spTree>
<dsp:nvGrpSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvGrpSpPr/></dsp:nvGrpSpPr><dsp:grpSpPr/>{inner}</dsp:spTree></dsp:drawing>"#
    )
}

fn good_drawing() -> String {
    drawing_xml(
        &[
            dsp_sp(100_000, 200_000, 1_000_000, 500_000, "Alpha"),
            dsp_sp(1_500_000, 900_000, 1_200_000, 600_000, "Beta"),
        ]
        .concat(),
    )
}

fn rels(entries: &[(&str, &str, &str)]) -> String {
    let body: String = entries
        .iter()
        .map(|(id, ty, t)| format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{t}"/>"#))
        .collect();
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
    )
}

/// 单 slide deck;`slide_rels` 是 slide 的关系,`extra` 是额外部件(diagrams/*)。
fn deck(sp_tree: &str, slide_rels: &[(&str, &str, &str)], extra: &[(&str, String)]) -> Vec<u8> {
    let mut parts: Vec<(String, String)> = vec![
        (
            "ppt/presentation.xml".into(),
            format!(
                r#"<p:presentation {NS}><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
<p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
            ),
        ),
        (
            "ppt/_rels/presentation.xml.rels".into(),
            rels(&[(
                "rId1",
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide",
                "slides/slide1.xml",
            )]),
        ),
        (
            "ppt/slides/slide1.xml".into(),
            format!(r#"<p:sld {NS}><p:cSld><p:spTree>{sp_tree}</p:spTree></p:cSld></p:sld>"#),
        ),
        ("ppt/slides/_rels/slide1.xml.rels".into(), rels(slide_rels)),
    ];
    parts.extend(extra.iter().map(|(n, b)| (n.to_string(), b.clone())));
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default();
        for (name, body) in &parts {
            zip.start_file(name.as_str(), opts).expect("start_file");
            zip.write_all(body.as_bytes()).expect("write");
        }
        zip.finish().expect("finish");
    }
    buf.into_inner()
}

const RELS_BOTH: [(&str, &str, &str); 2] = [
    ("rId2", REL_DATA, "../diagrams/data1.xml"),
    ("rId6", REL_DRAWING, "../diagrams/drawing1.xml"),
];

fn text_of(bytes: &[u8]) -> (String, String) {
    let parsed = parse_bytes(bytes).expect("parse");
    let resolved = resolve(&parsed);
    let opts = ExportOptions::default();
    (
        presentation_text_with(&parsed.presentation, Some(&resolved), &opts),
        presentation_markdown_with(&parsed.presentation, Some(&resolved), &opts),
    )
}

#[test]
fn drawing_part_becomes_group_placed_by_frame_transform() {
    let bytes = deck(
        &frame("rId2"),
        &RELS_BOTH,
        &[
            ("ppt/diagrams/data1.xml", data_xml(Some("rId6"))),
            ("ppt/diagrams/drawing1.xml", good_drawing()),
        ],
    );
    let parsed = parse_bytes(&bytes).expect("parse");
    let shapes = &parsed.presentation.slides[0].shapes;
    assert_eq!(shapes.len(), 1);
    let Shape::Group(g) = &shapes[0] else {
        panic!("expected group, got {:?}", shapes[0]);
    };
    // frame 矩形 = 组合矩形;drawing 坐标系 = 以 frame 左上为原点、同 frame 尺寸(恒等缩放)。
    assert_eq!(g.rect, Some(FRAME));
    assert_eq!(g.child_rect, Some(Rect::new(0, 0, FRAME.w, FRAME.h)));
    assert_eq!(g.children.len(), 2);
    let Shape::Auto(a) = &g.children[0] else {
        panic!("expected autoshape");
    };
    assert_eq!(
        a.rect,
        Some(Rect::new(100_000, 200_000, 1_000_000, 500_000))
    );
    assert_eq!(a.geometry.as_deref(), Some("roundRect"));
    assert!(a.style.is_some(), "dsp:style 与 p:style 同构");
    let tf = a.text.as_ref().expect("text");
    assert_eq!(tf.paragraphs[0].runs[0].text, "Alpha");

    let (text, md) = text_of(&bytes);
    assert!(text.contains("Alpha") && text.contains("Beta"), "{text}");
    assert!(md.contains("Alpha") && md.contains("Beta"), "{md}");
    // drawing 优先:不再叠加 data 部件的点文字(Gamma 只在 data 里)。
    assert!(!text.contains("Gamma"), "{text}");
}

#[test]
fn data_only_extracts_content_points_in_document_order_and_keeps_placeholder() {
    let bytes = deck(
        &frame("rId2"),
        &[("rId2", REL_DATA, "../diagrams/data1.xml")],
        &[("ppt/diagrams/data1.xml", data_xml(None))],
    );
    let parsed = parse_bytes(&bytes).expect("parse");
    let Shape::Placeholder(p) = &parsed.presentation.slides[0].shapes[0] else {
        panic!("expected placeholder");
    };
    assert_eq!(p.rect, Some(FRAME));
    assert_eq!(p.kind.as_deref(), Some(DGM));
    // 跳过 doc 空点 / pres / parTrans / sibTrans;空段落丢弃;`asst` 是内容点。
    assert_eq!(p.diagram_text, ["Alpha", "Beta", "Gamma", "Delta"]);

    let (text, md) = text_of(&bytes);
    assert!(text.contains("Alpha\nBeta\nGamma\nDelta"), "{text}");
    assert!(md.contains("Alpha") && md.contains("Delta"), "{md}");
    assert!(!text.contains("SENTINEL") && !md.contains("SENTINEL"));
}

#[test]
fn drawing_found_via_slide_rels_without_data_model_ext() {
    // data 部件没有 dataModelExt:按 dataN ↔ drawingN 的编号在 slide rels 里认 drawing。
    let bytes = deck(
        &frame("rId2"),
        &RELS_BOTH,
        &[
            ("ppt/diagrams/data1.xml", data_xml(None)),
            ("ppt/diagrams/drawing1.xml", good_drawing()),
        ],
    );
    let parsed = parse_bytes(&bytes).expect("parse");
    assert!(matches!(
        parsed.presentation.slides[0].shapes[0],
        Shape::Group(_)
    ));
}

#[test]
fn both_parts_missing_keeps_empty_placeholder() {
    let bytes = deck(&frame("rId2"), &RELS_BOTH, &[]);
    let parsed = parse_bytes(&bytes).expect("parse");
    let Shape::Placeholder(p) = &parsed.presentation.slides[0].shapes[0] else {
        panic!("expected placeholder");
    };
    assert_eq!(p.rect, Some(FRAME));
    assert!(p.diagram_text.is_empty());
    // 没有 relIds / 关系指向空气:同样不 panic。
    let bytes = deck(&frame(""), &[], &[]);
    assert!(parse_bytes(&bytes).is_ok());
    let bytes = deck(&frame("rId99"), &[], &[]);
    assert!(parse_bytes(&bytes).is_ok());
}

#[test]
fn malformed_or_empty_drawing_falls_back_to_data() {
    for bad in [
        "this is not xml at all".to_string(),
        "<dsp:drawing xmlns:dsp=\"urn:x\"><dsp:spTree><dsp:sp><dsp:spPr><a:xfrm".to_string(),
        drawing_xml(""), // 合法但没有任何形状
    ] {
        let bytes = deck(
            &frame("rId2"),
            &RELS_BOTH,
            &[
                ("ppt/diagrams/data1.xml", data_xml(Some("rId6"))),
                ("ppt/diagrams/drawing1.xml", bad),
            ],
        );
        let parsed = parse_bytes(&bytes).expect("parse");
        let Shape::Placeholder(p) = &parsed.presentation.slides[0].shapes[0] else {
            panic!("expected fallback to placeholder");
        };
        assert_eq!(p.diagram_text, ["Alpha", "Beta", "Gamma", "Delta"]);
    }
}

#[test]
fn deeply_nested_drawing_and_data_do_not_overflow_the_stack() {
    const N: usize = 100_000;
    let nested = format!("{}{}", "<dsp:grpSp>".repeat(N), "</dsp:grpSp>".repeat(N));
    let deep_data = format!(
        r#"<dgm:dataModel xmlns:dgm="{DGM}" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dgm:ptLst>
<dgm:pt><dgm:t><a:bodyPr/>{}<a:p><a:r><a:t>Deep</a:t></a:r></a:p>{}</dgm:t></dgm:pt></dgm:ptLst></dgm:dataModel>"#,
        "<a:x>".repeat(N),
        "</a:x>".repeat(N)
    );
    let bytes = deck(
        &frame("rId2"),
        &RELS_BOTH,
        &[
            ("ppt/diagrams/data1.xml", data_xml(Some("rId6"))),
            ("ppt/diagrams/drawing1.xml", drawing_xml(&nested)),
        ],
    );
    let (text, _) = text_of(&bytes);
    assert!(text.starts_with("--- slide 1 ---"));
    let bytes = deck(
        &frame("rId2"),
        &[("rId2", REL_DATA, "../diagrams/data1.xml")],
        &[("ppt/diagrams/data1.xml", deep_data)],
    );
    assert!(parse_bytes(&bytes).is_ok());
}

// --- 展开预算:小文件 + 多 frame 指向同一 drawing / data,总展开量必须有界 ---

/// 数一棵形状树(含组合内)里的形状总数(组合自身与降级留下的占位框不计)。
fn count_leaves(shapes: &[Shape]) -> usize {
    shapes
        .iter()
        .map(|s| match s {
            Shape::Group(g) => count_leaves(&g.children),
            Shape::Placeholder(_) => 0,
            _ => 1,
        })
        .sum()
}

/// `n_shapes` 个形状的 drawing × `n_frames` 个 frame,全部指向同一 data / drawing。
fn amplified(n_shapes: usize, n_frames: usize, data: String) -> Vec<u8> {
    let inner: String = (0..n_shapes)
        .map(|i| dsp_sp(0, 0, 100, 100, &format!("t{i}")))
        .collect();
    deck(
        &frame("rId2").repeat(n_frames),
        &RELS_BOTH,
        &[
            ("ppt/diagrams/data1.xml", data),
            ("ppt/diagrams/drawing1.xml", drawing_xml(&inner)),
        ],
    )
}

fn diag_count(parsed: &ppt_parse::ParsedPptx, kind: DiagnosticKind) -> usize {
    parsed
        .presentation
        .diagnostics
        .iter()
        .filter(|d| d.kind == kind)
        .map(|d| d.count)
        .sum()
}

#[test]
fn defaults_for_expansion_budgets() {
    let d = ZipLimits::default();
    assert_eq!(d.max_diagram_shapes, 100_000);
    assert_eq!(d.max_diagram_text_bytes, 8 * 1024 * 1024);
    assert_eq!(d.max_chart_points, 2_000_000);
}

/// 解析时预算的缺省值(单部件 < 总量;封住最坏内存的依据见 `ZipLimits` 文档)。
#[test]
fn defaults_for_parse_time_budgets() {
    let d = ZipLimits::default();
    assert_eq!((d.max_part_shapes, d.max_total_shapes), (20_000, 1_000_000));
    assert_eq!((d.max_part_items, d.max_total_items), (200_000, 8_000_000));
    assert_eq!(d.max_model_bytes, 2 * 1024 * 1024 * 1024);
    assert!(d.max_part_shapes < d.max_total_shapes);
    assert!(d.max_part_items < d.max_total_items);
}

#[test]
fn diagram_shape_budget_caps_total_expansion_across_frames() {
    // 200 形状 × 50 frame = 10 000;预算 1 000 => 只有 5 个 frame 能展开,其余 45 个降级。
    let limits = ZipLimits {
        max_diagram_shapes: 1_000,
        ..ZipLimits::default()
    };
    let parsed =
        parse_bytes_with_limits(&amplified(200, 50, data_xml(Some("rId6"))), &limits).unwrap();
    let shapes = &parsed.presentation.slides[0].shapes;
    assert_eq!(shapes.len(), 50, "frame 不丢,降级的保留占位框");
    assert_eq!(count_leaves(shapes), 5 * 200, "总展开量不超过预算");
    let groups = shapes
        .iter()
        .filter(|s| matches!(s, Shape::Group(_)))
        .count();
    assert_eq!(groups, 5);
    assert_eq!(diag_count(&parsed, DiagnosticKind::SmartArtDegraded), 45);
}

#[test]
fn diagram_budget_is_exact_boundary() {
    // 预算恰好够 3 个 frame:3 个展开,第 4 个降级。
    let limits = ZipLimits {
        max_diagram_shapes: 600,
        ..ZipLimits::default()
    };
    let parsed =
        parse_bytes_with_limits(&amplified(200, 4, data_xml(Some("rId6"))), &limits).unwrap();
    let shapes = &parsed.presentation.slides[0].shapes;
    assert_eq!(count_leaves(shapes), 600);
    assert_eq!(diag_count(&parsed, DiagnosticKind::SmartArtDegraded), 1);
}

#[test]
fn diagram_text_budget_caps_fallback_text_across_frames() {
    // 无 drawing,退回 data 文字:每个 frame 的文字 "Alpha Beta Gamma Delta" = 20 字节;
    // 预算 50 字节 => 只有前 2 个 frame 拿到文字。
    let limits = ZipLimits {
        max_diagram_text_bytes: 50,
        ..ZipLimits::default()
    };
    let bytes = deck(
        &frame("rId2").repeat(10),
        &[("rId2", REL_DATA, "../diagrams/data1.xml")],
        &[("ppt/diagrams/data1.xml", data_xml(None))],
    );
    let parsed = parse_bytes_with_limits(&bytes, &limits).unwrap();
    let with_text = parsed.presentation.slides[0]
        .shapes
        .iter()
        .filter(|s| matches!(s, Shape::Placeholder(gp) if !gp.diagram_text.is_empty()))
        .count();
    assert_eq!(with_text, 2);
}

#[test]
fn diagram_text_budget_also_charges_drawing_text() {
    // 每形状文字 2 字节("t0"…"t9" 共 10 个 = 20 字节),预算 50 => 2 个 frame 展开。
    let limits = ZipLimits {
        max_diagram_text_bytes: 50,
        ..ZipLimits::default()
    };
    let parsed =
        parse_bytes_with_limits(&amplified(10, 5, data_xml(Some("rId6"))), &limits).unwrap();
    let groups = parsed.presentation.slides[0]
        .shapes
        .iter()
        .filter(|s| matches!(s, Shape::Group(_)))
        .count();
    assert_eq!(groups, 2);
}

#[test]
fn oversized_single_drawing_is_degraded_even_with_huge_budget() {
    // 单个 drawing 部件的形状数有硬上限(10 000),与全局预算无关。
    let limits = ZipLimits {
        max_diagram_shapes: usize::MAX,
        ..ZipLimits::default()
    };
    let parsed =
        parse_bytes_with_limits(&amplified(10_001, 1, data_xml(Some("rId6"))), &limits).unwrap();
    assert!(matches!(
        parsed.presentation.slides[0].shapes[0],
        Shape::Placeholder(_)
    ));
    assert_eq!(diag_count(&parsed, DiagnosticKind::SmartArtDegraded), 1);
}
