//! 模型字节预算(`ZipLimits::max_model_bytes`)的验收性质:对一组"小文件、大展开"的输入族
//! (每个放大点一个生成器,规模参数可调),在小规模、小预算下断言
//! "估算的模型字节数 ≤ 预算",且展开需求超过预算时有诊断;预算充足时同一输入零诊断。

use std::io::{Cursor, Write};

use ppt_core::DiagnosticKind;
use ppt_parse::{estimated_model_bytes, parse_bytes_with_limits, ParsedPptx, ZipLimits};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const DGM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
const CHART: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

type Rel = (String, String, String);

fn rel(id: &str, ty: &str, target: &str) -> Rel {
    (id.to_string(), ty.to_string(), target.to_string())
}

fn rels(items: &[Rel]) -> String {
    let body: String = items
        .iter()
        .map(|(id, ty, t)| format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{t}"/>"#))
        .collect();
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
    )
}

/// `slides` 每项是 `(spTree 内容, rels)`;`extra` 是额外部件。
fn build(slides: &[(String, Vec<Rel>)], extra: &[(String, String)]) -> Vec<u8> {
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
    parts.extend(extra.iter().cloned());
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

fn text_box(inner: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/>{inner}</p:txBody></p:sp>"#
    )
}

fn frame(uri: &str, inner: &str) -> String {
    format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="F"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
<p:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></p:xfrm>
<a:graphic><a:graphicData uri="{uri}">{inner}</a:graphicData></a:graphic></p:graphicFrame>"#
    )
}

// ------------------------------------------------------------------ 生成器(规模参数可调)

/// 节点洪水:`n` 个 `<a:tc/>`(7 字节 → 一个 800+ 字节的 `Cell`)。
fn gen_cells(n: usize) -> Vec<u8> {
    let table = frame(
        "http://schemas.openxmlformats.org/drawingml/2006/table",
        &format!(
            r#"<a:tbl><a:tblGrid><a:gridCol w="1"/></a:tblGrid><a:tr h="1">{}</a:tr></a:tbl>"#,
            "<a:tc/>".repeat(n)
        ),
    );
    build(&[(table, vec![])], &[])
}

/// 形状洪水:`n` 个空连接线 × `slides` 页。
fn gen_shapes(n: usize, slides: usize) -> Vec<u8> {
    let s: Vec<_> = (0..slides)
        .map(|_| ("<p:cxnSp></p:cxnSp>".repeat(n), vec![]))
        .collect();
    build(&s, &[])
}

/// 长文本:`n` 个各 `len` 字节的 run。
fn gen_text(n: usize, len: usize) -> Vec<u8> {
    let run = format!("<a:r><a:t>{}</a:t></a:r>", "t".repeat(len));
    build(
        &[(text_box(&format!("<a:p>{}</a:p>", run.repeat(n))), vec![])],
        &[],
    )
}

/// 批注拷贝:一个 `n` 条(正文 `len` 字节)的部件被 `slides` 张幻灯片共享。
fn gen_comments(n: usize, len: usize, slides: usize) -> Vec<u8> {
    let cm = format!(
        r#"<p:cmLst xmlns:p="urn:p">{}</p:cmLst>"#,
        format!(r#"<p:cm><p:text>{}</p:text></p:cm>"#, "c".repeat(len)).repeat(n)
    );
    let s: Vec<_> = (0..slides)
        .map(|_| {
            (
                String::new(),
                vec![rel(
                    "rId1",
                    &format!("{REL}/comments"),
                    "../comments/comment1.xml",
                )],
            )
        })
        .collect();
    build(&s, &[("ppt/comments/comment1.xml".into(), cm)])
}

/// SmartArt 克隆:一个 `shapes` 个带文字形状的 drawing × `frames` 个 frame。
fn gen_smartart(shapes: usize, frames: usize) -> Vec<u8> {
    let f = frame(
        DGM,
        &format!(r#"<dgm:relIds xmlns:dgm="{DGM}" r:dm="rId2"/>"#),
    )
    .repeat(frames);
    let sp = r#"<dsp:sp><dsp:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="10" cy="10"/></a:xfrm></dsp:spPr><dsp:txBody><a:p><a:r><a:t>node text</a:t></a:r></a:p></dsp:txBody></dsp:sp>"#;
    let drawing = format!(
        r#"<dsp:drawing xmlns:dsp="urn:dsp" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dsp:spTree>{}</dsp:spTree></dsp:drawing>"#,
        sp.repeat(shapes)
    );
    let data = format!(r#"<dgm:dataModel xmlns:dgm="{DGM}"><dgm:ptLst/></dgm:dataModel>"#);
    build(
        &[(
            f,
            vec![
                rel(
                    "rId2",
                    &format!("{REL}/diagramData"),
                    "../diagrams/data1.xml",
                ),
                rel(
                    "rId6",
                    "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing",
                    "../diagrams/drawing1.xml",
                ),
            ],
        )],
        &[
            ("ppt/diagrams/data1.xml".into(), data),
            ("ppt/diagrams/drawing1.xml".into(), drawing),
        ],
    )
}

/// 图表补空:`ptCount` 声明 `pts` 个点、`series` 个系列(几十字节的 XML → 每点一个槽位)。
fn gen_chart_padding(series: usize, pts: usize) -> Vec<u8> {
    let ser = format!(
        r#"<c:ser><c:cat><c:strLit><c:ptCount val="{pts}"/></c:strLit></c:cat><c:val><c:numLit><c:ptCount val="{pts}"/></c:numLit></c:val></c:ser>"#
    );
    let chart = format!(
        r#"<c:chartSpace xmlns:c="{CHART}"><c:chart><c:plotArea><c:barChart>{}</c:barChart></c:plotArea></c:chart></c:chartSpace>"#,
        ser.repeat(series)
    );
    build(
        &[(
            frame(
                CHART,
                &format!(r#"<c:chart xmlns:c="{CHART}" r:id="rId9"/>"#),
            ),
            vec![rel("rId9", &format!("{REL}/chart"), "../charts/chart1.xml")],
        )],
        &[("ppt/charts/chart1.xml".into(), chart)],
    )
}

/// 母版 / 版式:`n` 个形状的母版(与幻灯片共享同一份模型字节预算)。
fn gen_master(n: usize) -> Vec<u8> {
    let tree = "<p:cxnSp></p:cxnSp>".repeat(n);
    build(
        &[(
            String::new(),
            vec![rel(
                "rId1",
                &format!("{REL}/slideLayout"),
                "../slideLayouts/slideLayout1.xml",
            )],
        )],
        &[
            (
                "ppt/slideLayouts/slideLayout1.xml".into(),
                format!(r#"<p:sldLayout {NS}><p:cSld><p:spTree/></p:cSld></p:sldLayout>"#),
            ),
            (
                "ppt/slideLayouts/_rels/slideLayout1.xml.rels".into(),
                rels(&[rel(
                    "rId1",
                    &format!("{REL}/slideMaster"),
                    "../slideMasters/slideMaster1.xml",
                )]),
            ),
            (
                "ppt/slideMasters/slideMaster1.xml".into(),
                format!(
                    r#"<p:sldMaster {NS}><p:cSld><p:spTree>{tree}</p:spTree></p:cSld></p:sldMaster>"#
                ),
            ),
        ],
    )
}

// ------------------------------------------------------------------ 性质

/// 内容丢失类诊断的总计数(字节预算耗尽时至少有一种)。
fn loss(p: &ParsedPptx) -> usize {
    p.presentation
        .diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d.kind,
                DiagnosticKind::ContentTruncated
                    | DiagnosticKind::ShapesTruncated
                    | DiagnosticKind::ValueTruncated
                    | DiagnosticKind::CommentsTruncated
                    | DiagnosticKind::SmartArtDegraded
                    | DiagnosticKind::ChartDegraded
            )
        })
        .map(|d| d.count)
        .sum()
}

/// 对一个输入:小预算下估算字节 ≤ 预算且有诊断;宽松预算下零丢失,且估算超过小预算
/// (证明这个输入确实需要更多字节)。
fn check(name: &str, bytes: &[u8], budget: usize) {
    let tight = ZipLimits {
        max_model_bytes: budget,
        ..ZipLimits::default()
    };
    let p = parse_bytes_with_limits(bytes, &tight).unwrap();
    let est = estimated_model_bytes(&p);
    assert!(est <= budget, "{name}: 估算 {est} > 预算 {budget}");
    assert!(loss(&p) > 0, "{name}: 超预算必须有诊断");

    let full = parse_bytes_with_limits(bytes, &ZipLimits::default()).unwrap();
    assert_eq!(loss(&full), 0, "{name}: 缺省预算下零丢失");
    assert!(
        estimated_model_bytes(&full) > budget,
        "{name}: 生成器规模不足以超出 {budget}"
    );
}

#[test]
fn every_amplification_family_stays_within_the_model_byte_budget() {
    const B: usize = 256 * 1024;
    check("cells", &gen_cells(2_000), B);
    check("shapes", &gen_shapes(300, 3), B);
    check("text", &gen_text(100, 4_000), B);
    check("comments", &gen_comments(200, 500, 10), B);
    check("smartart", &gen_smartart(20, 60), B);
    check("chart-padding", &gen_chart_padding(4, 50_000), B);
    check("master", &gen_master(1_000), B);
}

/// 输入都是小文件(压缩后),但缺省预算下的模型字节是输入的数十到数百倍——正是预算要封住的形状。
#[test]
fn generators_are_small_inputs_with_large_expansion() {
    for (name, bytes) in [
        ("cells", gen_cells(2_000)),
        ("smartart", gen_smartart(20, 60)),
        ("chart-padding", gen_chart_padding(4, 50_000)),
    ] {
        let p = parse_bytes_with_limits(&bytes, &ZipLimits::default()).unwrap();
        let est = estimated_model_bytes(&p);
        assert!(
            est > 20 * bytes.len(),
            "{name}: 输入 {} 估算 {est}",
            bytes.len()
        );
    }
}

/// 预算是精确的:边界处恰好够用的预算下零丢失,少一个字节就有诊断。
#[test]
fn byte_budget_boundary_is_exact() {
    let bytes = gen_text(20, 100);
    let probe = ZipLimits {
        max_model_bytes: usize::MAX,
        ..ZipLimits::default()
    };
    // 找到恰好够用的预算:二分(解析是确定性的)。
    let lossless = |b: usize| {
        let p = parse_bytes_with_limits(
            &bytes,
            &ZipLimits {
                max_model_bytes: b,
                ..probe
            },
        )
        .unwrap();
        loss(&p) == 0
    };
    let (mut lo, mut hi) = (0usize, 1usize << 24);
    assert!(lossless(hi));
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if lossless(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    assert!(lossless(hi) && !lossless(hi - 1));
}

// ------------------------------------------------------------------ 继承链解析(终态 IR)

/// `slides` 页都引用同一个带 `n` 个装饰形状的母版。
fn gen_master_pages(n: usize, slides: usize) -> Vec<u8> {
    let tree = "<p:cxnSp></p:cxnSp>".repeat(n);
    let layout_rel = vec![rel(
        "rId1",
        &format!("{REL}/slideLayout"),
        "../slideLayouts/slideLayout1.xml",
    )];
    let s: Vec<_> = (0..slides)
        .map(|_| (String::new(), layout_rel.clone()))
        .collect();
    build(
        &s,
        &[
            (
                "ppt/slideLayouts/slideLayout1.xml".into(),
                format!(r#"<p:sldLayout {NS}><p:cSld><p:spTree/></p:cSld></p:sldLayout>"#),
            ),
            (
                "ppt/slideLayouts/_rels/slideLayout1.xml.rels".into(),
                rels(&[rel(
                    "rId1",
                    &format!("{REL}/slideMaster"),
                    "../slideMasters/slideMaster1.xml",
                )]),
            ),
            (
                "ppt/slideMasters/slideMaster1.xml".into(),
                format!(
                    r#"<p:sldMaster {NS}><p:cSld><p:spTree>{tree}</p:spTree></p:cSld></p:sldMaster>"#
                ),
            ),
        ],
    )
}

/// 母版形状逐页物化进终态 IR:全文实例数有上限,超出的计入 `inherited_dropped`。
#[test]
fn inherited_shapes_are_capped_across_the_deck() {
    let p = parse_bytes_with_limits(&gen_master_pages(100, 30), &ZipLimits::default()).unwrap();
    let r = ppt_parse::resolve_parts_capped(&p.presentation, &p.inherit, 1_000);
    let n: usize = r.slides.iter().map(|s| s.inherited_shapes.len()).sum();
    assert_eq!(n, 1_000);
    assert_eq!(r.inherited_dropped, 2_000);
    let full = ppt_parse::resolve(&p);
    assert_eq!(full.inherited_dropped, 0);
    assert_eq!(
        full.slides
            .iter()
            .map(|s| s.inherited_shapes.len())
            .sum::<usize>(),
        3_000
    );
}

/// 终态 IR 里同一张图表(配色终端化结果也相同)被所有 frame 共享,不逐 frame 复制。
#[test]
fn resolved_charts_are_shared_across_frames() {
    let chart = format!(
        r#"<c:chartSpace xmlns:c="{CHART}"><c:chart><c:plotArea><c:barChart><c:ser><c:spPr><a:solidFill xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:schemeClr val="accent1"/></a:solidFill></c:spPr>
<c:cat><c:strLit><c:ptCount val="1"/><c:pt idx="0"><c:v>K</c:v></c:pt></c:strLit></c:cat>
<c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#
    );
    let frames = frame(
        CHART,
        &format!(r#"<c:chart xmlns:c="{CHART}" r:id="rId9"/>"#),
    )
    .repeat(20);
    let bytes = build(
        &[(
            frames,
            vec![rel("rId9", &format!("{REL}/chart"), "../charts/chart1.xml")],
        )],
        &[("ppt/charts/chart1.xml".into(), chart)],
    );
    let p = parse_bytes_with_limits(&bytes, &ZipLimits::default()).unwrap();
    let r = ppt_parse::resolve(&p);
    let charts: Vec<_> = r.slides[0]
        .shapes
        .iter()
        .filter_map(|s| match s {
            ppt_core::resolved::ResolvedShape::Placeholder(g) => g.chart.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(charts.len(), 20);
    assert!(charts.iter().all(|c| std::sync::Arc::ptr_eq(c, &charts[0])));
    // 配色已终端化为显式 sRGB。
    assert!(matches!(
        charts[0].series[0].color,
        Some(ppt_core::color::ColorSpec::Srgb { .. })
    ));
}
