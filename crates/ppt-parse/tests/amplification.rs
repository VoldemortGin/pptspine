//! "放大攻击"回归:每个用例构造一个**小文件、大展开**的输入(审查探针的缩小版),断言的是
//! **有界性**——展开后的形状数 / 数据点数 / 批注数 / 诊断条数不超过对应上限,且超限时有诊断。
//! 规模控制在 1 秒量级;只断言计数,不依赖墙钟。pptx 现场合成,不落二进制 fixture。
//! (饼图图例条目数的有界性在 `ppt-render/src/chart.rs` 的单测里,那是渲染侧。)

use std::io::{Cursor, Write};

use ppt_core::model::{GraphicPlaceholder, Shape};
use ppt_core::DiagnosticKind;
use ppt_parse::{parse_bytes, ParsedPptx, ZipLimits};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const DGM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
const CHART: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const REL_DRAWING: &str = "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing";

/// 关系条目 `(Id, Type, Target)`。
type Rel = (String, String, String);

fn rels(items: &[Rel]) -> String {
    let body: String = items
        .iter()
        .map(|(id, ty, t)| format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{t}"/>"#))
        .collect();
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
    )
}

fn rel(id: &str, ty: &str, target: &str) -> Rel {
    (id.to_string(), ty.to_string(), target.to_string())
}

/// `slides` 里每项是 `(spTree 内容, 该 slide 的 rels)`;`extra` 是额外部件。
fn build(slides: &[(String, Vec<Rel>)], extra: &[(&str, String)]) -> Vec<u8> {
    let ids: String = (1..=slides.len())
        .map(|i| format!(r#"<p:sldId id="{}" r:id="rId{i}"/>"#, 255 + i))
        .collect();
    let pres_rels: Vec<_> = (1..=slides.len())
        .map(|i| {
            rel(
                &format!("rId{i}"),
                &format!("{REL}/slide"),
                &format!("slides/slide{i}.xml"),
            )
        })
        .collect();
    let mut parts: Vec<(String, String)> = vec![
        (
            "ppt/presentation.xml".into(),
            format!(
                r#"<p:presentation {NS}><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
            ),
        ),
        ("ppt/_rels/presentation.xml.rels".into(), rels(&pres_rels)),
    ];
    for (i, (tree, srels)) in slides.iter().enumerate() {
        let n = i + 1;
        parts.push((
            format!("ppt/slides/slide{n}.xml"),
            format!(r#"<p:sld {NS}><p:cSld><p:spTree>{tree}</p:spTree></p:cSld></p:sld>"#),
        ));
        parts.push((format!("ppt/slides/_rels/slide{n}.xml.rels"), rels(srels)));
    }
    parts.extend(extra.iter().map(|(n, b)| (n.to_string(), b.clone())));
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        for (name, body) in &parts {
            zip.start_file(name.as_str(), SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buf.into_inner()
}

fn diag_total(p: &ParsedPptx, kind: DiagnosticKind) -> usize {
    p.presentation
        .diagnostics
        .iter()
        .filter(|d| d.kind == kind)
        .map(|d| d.count)
        .sum()
}

fn leaves(shapes: &[Shape]) -> usize {
    shapes
        .iter()
        .map(|s| match s {
            Shape::Group(g) => leaves(&g.children),
            Shape::Placeholder(_) => 0,
            _ => 1,
        })
        .sum()
}

fn frame(uri: &str, inner: &str) -> String {
    format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="F"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
<p:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></p:xfrm>
<a:graphic><a:graphicData uri="{uri}">{inner}</a:graphicData></a:graphic></p:graphicFrame>"#
    )
}

/// SmartArt:1 000 个空 `dsp:cxnSp` 的 drawing × 150 个 frame(15 万,超默认预算 10 万)。
#[test]
fn smartart_fan_out_is_bounded_by_the_default_shape_budget() {
    let limits = ZipLimits::default();
    let frames = frame(
        DGM,
        &format!(r#"<dgm:relIds xmlns:dgm="{DGM}" r:dm="rId2"/>"#),
    )
    .repeat(150);
    let drawing = format!(
        r#"<dsp:drawing xmlns:dsp="urn:dsp"><dsp:spTree>{}</dsp:spTree></dsp:drawing>"#,
        "<dsp:cxnSp></dsp:cxnSp>".repeat(1_000)
    );
    let data = format!(r#"<dgm:dataModel xmlns:dgm="{DGM}"><dgm:ptLst/></dgm:dataModel>"#);
    let bytes = build(
        &[(
            frames,
            vec![
                rel(
                    "rId2",
                    &format!("{REL}/diagramData"),
                    "../diagrams/data1.xml",
                ),
                rel("rId6", REL_DRAWING, "../diagrams/drawing1.xml"),
            ],
        )],
        &[
            ("ppt/diagrams/data1.xml", data),
            ("ppt/diagrams/drawing1.xml", drawing),
        ],
    );
    assert!(bytes.len() < 20_000, "输入必须是小文件:{}", bytes.len());
    let p = parse_bytes(&bytes).unwrap();
    let expanded = leaves(&p.presentation.slides[0].shapes);
    assert!(expanded <= limits.max_diagram_shapes, "展开 {expanded}");
    assert_eq!(expanded, 100_000, "预算内的 frame 照常展开");
    assert_eq!(diag_total(&p, DiagnosticKind::SmartArtDegraded), 50);
    assert_eq!(p.presentation.slides[0].shapes.len(), 150, "frame 不丢");
}

/// 图表:`ptCount` 填满 2 万点的图表 × 100 个 frame(200 万,超默认预算 100 万)。
#[test]
fn chart_fan_out_is_bounded_by_the_default_point_budget() {
    let limits = ZipLimits::default();
    let frames = frame(
        CHART,
        &format!(r#"<c:chart xmlns:c="{CHART}" r:id="rId1"/>"#),
    )
    .repeat(100);
    let chart = format!(
        r#"<c:chartSpace xmlns:c="{CHART}"><c:chart><c:plotArea><c:barChart><c:ser><c:val><c:numLit><c:ptCount val="20000"/></c:numLit></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#
    );
    let bytes = build(
        &[(
            frames,
            vec![rel("rId1", &format!("{REL}/chart"), "../charts/chart1.xml")],
        )],
        &[("ppt/charts/chart1.xml", chart)],
    );
    let p = parse_bytes(&bytes).unwrap();
    let points: usize = p.presentation.slides[0]
        .shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Placeholder(GraphicPlaceholder { chart: Some(c), .. }) => {
                Some(c.categories.len() + c.series.iter().map(|s| s.values.len()).sum::<usize>())
            }
            _ => None,
        })
        .sum();
    assert!(points <= limits.max_chart_points, "展开 {points} 点");
    assert_eq!(points, 1_000_000);
    assert_eq!(diag_total(&p, DiagnosticKind::ChartDegraded), 50);
}

/// 批注:1 000 条的部件被 120 张幻灯片共享、每张还有 3 条重复关系(默认预算 10 万)。
#[test]
fn comment_fan_out_is_bounded_by_dedup_cache_and_the_default_budget() {
    let limits = ZipLimits::default();
    let cm = format!(
        r#"<p:cmLst xmlns:p="urn:p">{}</p:cmLst>"#,
        r#"<p:cm authorId="0"><p:text>hello</p:text></p:cm>"#.repeat(1_000)
    );
    let srels: Vec<_> = (0..3)
        .map(|i| {
            rel(
                &format!("rId{i}"),
                &format!("{REL}/comments"),
                "../comments/comment1.xml",
            )
        })
        .collect();
    let slides: Vec<_> = (0..120).map(|_| (String::new(), srels.clone())).collect();
    let p = parse_bytes(&build(&slides, &[("ppt/comments/comment1.xml", cm)])).unwrap();
    let total: usize = p.presentation.slides.iter().map(|s| s.comments.len()).sum();
    assert!(total <= limits.max_comments, "批注共 {total} 条");
    assert_eq!(total, 100_000);
    assert_eq!(diag_total(&p, DiagnosticKind::DuplicateCommentRef), 240);
    assert!(diag_total(&p, DiagnosticKind::CommentsTruncated) > 0);
    // 每张幻灯片内同一部件只取一份。
    assert!(p
        .presentation
        .slides
        .iter()
        .all(|s| s.comments.len() <= 1_000));
}

/// 诊断:5 000 个 frame 各指向一个不同名的不存在部件 => 诊断条数与目标数无关,
/// `part` 里不出现文件作者写的任意 Target 字符串,总计数不丢。
#[test]
fn diagnostics_stay_bounded_and_never_echo_attacker_targets() {
    let n = 5_000;
    let frames: String = (0..n)
        .map(|i| {
            frame(
                DGM,
                &format!(r#"<dgm:relIds xmlns:dgm="{DGM}" r:dm="rId{i}"/>"#),
            )
        })
        .collect();
    let srels: Vec<_> = (0..n)
        .map(|i| {
            rel(
                &format!("rId{i}"),
                &format!("{REL}/diagramData"),
                &format!("../diagrams/EVIL-{i}.xml"),
            )
        })
        .collect();
    let p = parse_bytes(&build(&[(frames, srels)], &[])).unwrap();
    let diags = &p.presentation.diagnostics;
    assert!(diags.len() <= 4, "诊断条数 {}", diags.len());
    assert_eq!(diag_total(&p, DiagnosticKind::SmartArtDegraded), n);
    assert!(diags.iter().all(|d| d.part == "ppt/slides/slide1.xml"));
    assert!(!format!("{diags:?}").contains("EVIL"));
}
