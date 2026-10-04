//! "放大攻击"回归:每个用例构造一个**小文件、大展开**的输入(审查探针的缩小版),断言的是
//! **有界性**——展开后的形状数 / 数据点数 / 批注数 / 诊断条数不超过对应上限,且超限时有诊断。
//! 规模控制在 1 秒量级;只断言计数,不依赖墙钟。pptx 现场合成,不落二进制 fixture。
//! (饼图图例条目数的有界性在 `ppt-render/src/chart.rs` 的单测里,那是渲染侧。)

use std::io::{Cursor, Write};

use ppt_core::model::{GraphicPlaceholder, Shape};
use ppt_core::DiagnosticKind;
use ppt_parse::{parse_bytes, parse_bytes_with_limits, ParsedPptx, ZipLimits};
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

/// 图表:`ptCount` 填满 2 万点的图表 × 150 个 frame(300 万,超默认预算 200 万)。
#[test]
fn chart_fan_out_is_bounded_by_the_default_point_budget() {
    let limits = ZipLimits::default();
    let frames = frame(
        CHART,
        &format!(r#"<c:chart xmlns:c="{CHART}" r:id="rId1"/>"#),
    )
    .repeat(150);
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
    assert_eq!(points, 2_000_000);
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

/// 批注部件只**解析**一次(缓存):嵌套超限诊断是解析期产生的,被 120 张幻灯片共享的部件只该记一次。
/// (没有缓存时输出的批注条数不变,只有这类"每次解析都会记一遍"的副作用能暴露它。)
#[test]
fn shared_comment_part_is_parsed_once_across_slides() {
    let mut inner = "<m:r><m:t>X</m:t></m:r>".to_string();
    for _ in 0..70 {
        inner =
            format!("<m:sSup><m:e>{inner}</m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup>");
    }
    let cm = format!(
        r#"<p188:cmLst xmlns:p188="urn:p188" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><p188:cm><p188:txBody><a:bodyPr/><a:p><a14:m xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main"><m:oMathPara><m:oMath>{inner}</m:oMath></m:oMathPara></a14:m></a:p></p188:txBody></p188:cm></p188:cmLst>"#
    );
    let srels = vec![rel(
        "rId0",
        &format!("{REL}/comments"),
        "../comments/comment1.xml",
    )];
    let slides: Vec<_> = (0..120).map(|_| (String::new(), srels.clone())).collect();
    let p = parse_bytes(&build(&slides, &[("ppt/comments/comment1.xml", cm)])).unwrap();
    assert_eq!(
        diag_at(
            &p,
            DiagnosticKind::NestingTooDeep,
            "ppt/comments/comment1.xml"
        ),
        1
    );
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

// ---------------------------------------------------------------- 解析时的形状 / 节点预算

const SLIDE1: &str = "ppt/slides/slide1.xml";

/// `part` 上某种诊断的总计数。
fn diag_at(p: &ParsedPptx, kind: DiagnosticKind, part: &str) -> usize {
    p.presentation
        .diagnostics
        .iter()
        .filter(|d| d.kind == kind && d.part == part)
        .map(|d| d.count)
        .sum()
}

/// 一个"短 XML、大模型"的形状:`<p:cxnSp>` 恒产出一个 `Connector`(`<p:sp>` 无内容会被丢掉)。
const TINY_SHAPE: &str = "<p:cxnSp></p:cxnSp>";

fn text_box(inner: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/>{inner}</p:txBody></p:sp>"#
    )
}

fn paragraphs_of(p: &ParsedPptx) -> usize {
    p.presentation.slides[0]
        .shapes
        .iter()
        .map(|s| match s {
            Shape::TextBox(t) => t.paragraphs.len(),
            _ => 0,
        })
        .sum()
}

/// 单部件形状数:2.5 万个 `<p:cxnSp></p:cxnSp>`(约 0.5 MB)被默认单部件预算(2 万)截住,
/// 已解析的保留,丢弃数记进 `shapes-truncated`。
#[test]
fn shape_flood_in_one_slide_is_bounded_by_the_default_part_budget() {
    let limits = ZipLimits::default();
    let bytes = build(&[(TINY_SHAPE.repeat(25_000), vec![])], &[]);
    assert!(bytes.len() < 100_000, "输入必须是小文件:{}", bytes.len());
    let p = parse_bytes(&bytes).unwrap();
    let n = p.presentation.slides[0].shapes.len();
    assert_eq!(n, limits.max_part_shapes);
    assert_eq!(diag_at(&p, DiagnosticKind::ShapesTruncated, SLIDE1), 5_000);
}

/// 演示文稿级总预算:每页都在单部件预算之内,累计超过总预算。截断是公平的——每页保底
/// `min(100, 总额 / 页数)` = 50 个,不是前两页完整、后两页为空。
#[test]
fn shape_flood_across_slides_is_bounded_by_the_total_budget() {
    let limits = ZipLimits {
        max_part_shapes: 100,
        max_total_shapes: 250,
        ..ZipLimits::default()
    };
    let slides: Vec<_> = (0..5).map(|_| (TINY_SHAPE.repeat(100), vec![])).collect();
    let p = parse_bytes_with_limits(&build(&slides, &[]), &limits).unwrap();
    let per_slide: Vec<usize> = p
        .presentation
        .slides
        .iter()
        .map(|s| s.shapes.len())
        .collect();
    assert_eq!(per_slide, [50, 50, 50, 50, 50]);
    assert_eq!(per_slide.iter().sum::<usize>(), limits.max_total_shapes);
    let at = |i: usize| {
        diag_at(
            &p,
            DiagnosticKind::ShapesTruncated,
            &format!("ppt/slides/slide{i}.xml"),
        )
    };
    assert_eq!((at(1), at(2), at(3), at(4), at(5)), (50, 50, 50, 50, 50));
}

/// 组合里的后代同样计数:嵌套的形状洪水不能绕过预算。
#[test]
fn shapes_inside_groups_count_against_the_budget() {
    let group = format!(
        r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="9" name="G"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{}</p:grpSp>"#,
        TINY_SHAPE.repeat(500)
    );
    let limits = ZipLimits {
        max_part_shapes: 101, // 组合自己 1 个 + 100 个后代
        ..ZipLimits::default()
    };
    let p = parse_bytes_with_limits(&build(&[(group, vec![])], &[]), &limits).unwrap();
    assert_eq!(leaves(&p.presentation.slides[0].shapes), 100);
    assert_eq!(diag_at(&p, DiagnosticKind::ShapesTruncated, SLIDE1), 400);
}

/// 版式 / 母版也受演示文稿级总预算约束(与 slide 累计)。
#[test]
fn layout_and_master_shapes_share_the_total_budget() {
    let layout = format!(
        r#"<p:sldLayout {NS}><p:cSld><p:spTree>{}</p:spTree></p:cSld></p:sldLayout>"#,
        TINY_SHAPE.repeat(100)
    );
    let master = format!(
        r#"<p:sldMaster {NS}><p:cSld><p:spTree>{}</p:spTree></p:cSld></p:sldMaster>"#,
        TINY_SHAPE.repeat(100)
    );
    let limits = ZipLimits {
        max_part_shapes: 100,
        max_total_shapes: 250,
        ..ZipLimits::default()
    };
    let bytes = build(
        &[(
            TINY_SHAPE.repeat(100),
            vec![rel(
                "rId1",
                &format!("{REL}/slideLayout"),
                "../slideLayouts/slideLayout1.xml",
            )],
        )],
        &[
            ("ppt/slideLayouts/slideLayout1.xml", layout),
            (
                "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
                rels(&[rel(
                    "rId1",
                    &format!("{REL}/slideMaster"),
                    "../slideMasters/slideMaster1.xml",
                )]),
            ),
            ("ppt/slideMasters/slideMaster1.xml", master),
        ],
    );
    let p = parse_bytes_with_limits(&bytes, &limits).unwrap();
    assert_eq!(p.presentation.slides[0].shapes.len(), 100);
    let layout_n: usize = p.inherit.layouts.values().map(|l| l.shapes.len()).sum();
    let master_n: usize = p.inherit.masters.values().map(|m| m.shapes.len()).sum();
    assert_eq!((layout_n, master_n), (100, 50));
    assert_eq!(
        diag_at(
            &p,
            DiagnosticKind::ShapesTruncated,
            "ppt/slideMasters/slideMaster1.xml"
        ),
        50
    );
}

/// SmartArt drawing 部件本身的形状洪水:解析时截断(保留前 N 个),与展开预算互相独立。
#[test]
fn smartart_drawing_flood_is_truncated_at_parse_time() {
    let limits = ZipLimits {
        max_part_shapes: 100,
        ..ZipLimits::default()
    };
    let frames = frame(
        DGM,
        &format!(r#"<dgm:relIds xmlns:dgm="{DGM}" r:dm="rId2"/>"#),
    );
    let drawing = format!(
        r#"<dsp:drawing xmlns:dsp="urn:dsp"><dsp:spTree>{}</dsp:spTree></dsp:drawing>"#,
        "<dsp:cxnSp></dsp:cxnSp>".repeat(5_000)
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
    let p = parse_bytes_with_limits(&bytes, &limits).unwrap();
    assert_eq!(leaves(&p.presentation.slides[0].shapes), 100);
    assert_eq!(
        diag_at(
            &p,
            DiagnosticKind::ShapesTruncated,
            "ppt/diagrams/drawing1.xml"
        ),
        4_900
    );
}

/// 文本节点:单个文本框 25 万个 `<a:p/>`(约 1.7 MB)被默认单部件节点预算(20 万)截住。
#[test]
fn paragraph_flood_is_bounded_by_the_default_item_budget() {
    let limits = ZipLimits::default();
    let bytes = build(&[(text_box(&"<a:p/>".repeat(250_000)), vec![])], &[]);
    assert!(bytes.len() < 2_000_000, "输入必须是小文件:{}", bytes.len());
    let p = parse_bytes(&bytes).unwrap();
    assert_eq!(paragraphs_of(&p), limits.max_part_items);
    assert_eq!(
        diag_at(&p, DiagnosticKind::ContentTruncated, SLIDE1),
        50_000
    );
}

/// run 洪水:一个段落里的 `<a:br/>` / `<a:r>` 也计入节点预算。
#[test]
fn run_flood_is_bounded_by_the_item_budget() {
    let limits = ZipLimits {
        max_part_items: 1_000,
        ..ZipLimits::default()
    };
    let para = format!(
        "<a:p>{}{}</a:p>",
        "<a:br/>".repeat(800),
        "<a:r><a:t>x</a:t></a:r>".repeat(800)
    );
    let p = parse_bytes_with_limits(&build(&[(text_box(&para), vec![])], &[]), &limits).unwrap();
    let Shape::TextBox(t) = &p.presentation.slides[0].shapes[0] else {
        panic!("expected a text box");
    };
    // 1 个段落 + 999 个 run = 1 000 个节点。
    assert_eq!(t.paragraphs.len(), 1);
    assert_eq!(t.paragraphs[0].runs.len(), 999);
    assert_eq!(diag_at(&p, DiagnosticKind::ContentTruncated, SLIDE1), 601);
}

/// 表格:行 / 单元格 / 网格列都计入节点预算(`<a:tc/>` 只有 7 字节,模型里一个 `Cell` 800+ 字节)。
#[test]
fn table_flood_is_bounded_by_the_item_budget() {
    let limits = ZipLimits {
        max_part_items: 5_000,
        ..ZipLimits::default()
    };
    let grid = "<a:gridCol w=\"1\"/>".repeat(1_000);
    let rows = format!("<a:tr h=\"1\">{}</a:tr>", "<a:tc/>".repeat(1_000)).repeat(50);
    let table = format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="T"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
<p:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></p:xfrm>
<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblGrid>{grid}</a:tblGrid>{rows}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#
    );
    let p = parse_bytes_with_limits(&build(&[(table, vec![])], &[]), &limits).unwrap();
    let Shape::Table(t) = &p.presentation.slides[0].shapes[0] else {
        panic!("expected a table");
    };
    let nodes =
        t.col_widths.len() + t.rows.len() + t.rows.iter().map(|r| r.cells.len()).sum::<usize>();
    assert_eq!(nodes, limits.max_part_items);
    assert!(diag_at(&p, DiagnosticKind::ContentTruncated, SLIDE1) > 0);
}

/// 演示文稿级节点总预算:每页在单部件预算内,累计超出。截断公平:每页保底
/// `min(2000, 250 / 4)` = 62 个节点,首页多拿到余数。
#[test]
fn item_flood_across_slides_is_bounded_by_the_total_budget() {
    let limits = ZipLimits {
        max_part_items: 100,
        max_total_items: 250,
        ..ZipLimits::default()
    };
    let slides: Vec<_> = (0..4)
        .map(|_| (text_box(&"<a:p/>".repeat(100)), vec![]))
        .collect();
    let p = parse_bytes_with_limits(&build(&slides, &[]), &limits).unwrap();
    let per_slide: Vec<usize> = p
        .presentation
        .slides
        .iter()
        .map(|s| match &s.shapes[0] {
            Shape::TextBox(t) => t.paragraphs.len(),
            _ => 0,
        })
        .collect();
    assert_eq!(per_slide, [64, 62, 62, 62]);
}

/// 默认预算下正常文档不受影响:几百个带文字的形状 + 一张表,没有任何截断诊断。
#[test]
fn normal_documents_are_unaffected_by_the_default_budgets() {
    let boxes: String = (0..300)
        .map(|i| {
            text_box(&format!(
                "<a:p><a:r><a:t>line {i}</a:t></a:r><a:br/><a:r><a:t>more</a:t></a:r></a:p>"
            ))
        })
        .collect();
    let p = parse_bytes(&build(
        &[(boxes, vec![]), (TINY_SHAPE.repeat(50), vec![])],
        &[],
    ))
    .unwrap();
    assert_eq!(p.presentation.slides[0].shapes.len(), 300);
    assert_eq!(p.presentation.slides[1].shapes.len(), 50);
    assert_eq!(diag_total(&p, DiagnosticKind::ShapesTruncated), 0);
    assert_eq!(diag_total(&p, DiagnosticKind::ContentTruncated), 0);
}

/// 主题的 `a:lnStyleLst` 里 25 万个 `<a:ln w="1"/>`(1.5 MB)也被默认节点预算截住
/// (`parse_part` 之外的部件同样是放大点)。
#[test]
fn theme_style_flood_is_bounded_by_the_default_item_budget() {
    let theme = format!(
        r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:fmtScheme name="x"><a:lnStyleLst>{}</a:lnStyleLst></a:fmtScheme></a:themeElements></a:theme>"#,
        r#"<a:ln w="1"/>"#.repeat(250_000)
    );
    let layout =
        format!(r#"<p:sldLayout {NS}><p:cSld><p:spTree></p:spTree></p:cSld></p:sldLayout>"#);
    let master =
        format!(r#"<p:sldMaster {NS}><p:cSld><p:spTree></p:spTree></p:cSld></p:sldMaster>"#);
    let bytes = build(
        &[(
            String::new(),
            vec![rel(
                "rId1",
                &format!("{REL}/slideLayout"),
                "../slideLayouts/slideLayout1.xml",
            )],
        )],
        &[
            ("ppt/slideLayouts/slideLayout1.xml", layout),
            (
                "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
                rels(&[rel(
                    "rId1",
                    &format!("{REL}/slideMaster"),
                    "../slideMasters/slideMaster1.xml",
                )]),
            ),
            ("ppt/slideMasters/slideMaster1.xml", master),
            (
                "ppt/slideMasters/_rels/slideMaster1.xml.rels",
                rels(&[rel("rId1", &format!("{REL}/theme"), "../theme/theme1.xml")]),
            ),
            ("ppt/theme/theme1.xml", theme),
        ],
    );
    let p = parse_bytes(&bytes).unwrap();
    let lines: usize = p.inherit.themes.values().map(|t| t.line_styles.len()).sum();
    assert_eq!(lines, ZipLimits::default().max_part_items);
}
