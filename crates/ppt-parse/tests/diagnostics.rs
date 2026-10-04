//! 解析诊断通道验收:`Presentation.diagnostics` 记录"内容被静默丢失 / 降级"的结构化事实
//! (种类 + 部件路径 + 计数,**绝不含文档正文**):部件 XML 中途损坏被截断、嵌套超限被跳过、
//! 重复幻灯片引用被去重、关系指向缺失部件、SmartArt / 图表降级。完全正常的文件为空。
//! pptx 现场合成。

use std::io::{Cursor, Write};

use ppt_core::{Diagnostic, DiagnosticKind};
use ppt_parse::{parse_bytes, ParsedPptx};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const SECRET: &str = "SECRET-BODY-TEXT";
const SLIDE: &str = "ppt/slides/slide1.xml";

fn rels(entries: &[(&str, &str, &str)]) -> String {
    let body: String = entries
        .iter()
        .map(|(id, ty, t)| format!(r#"<Relationship Id="{id}" Type="{REL}/{ty}" Target="{t}"/>"#))
        .collect();
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
    )
}

fn text_sp(text: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
<p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm></p:spPr>
<p:txBody><a:bodyPr/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"#
    )
}

fn slide_xml(sp_tree: &str) -> String {
    format!(r#"<p:sld {NS}><p:cSld><p:spTree>{sp_tree}</p:spTree></p:cSld></p:sld>"#)
}

fn presentation(rids: &[&str]) -> String {
    let ids: String = rids
        .iter()
        .enumerate()
        .map(|(i, r)| format!(r#"<p:sldId id="{}" r:id="{r}"/>"#, 256 + i))
        .collect();
    format!(
        r#"<p:presentation {NS}><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
    )
}

fn zip(parts: &[(&str, String)]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = ZipWriter::new(&mut buf);
        for (name, body) in parts {
            z.start_file(*name, SimpleFileOptions::default()).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

/// 单 slide 包;`slide` 是 slide1.xml 全文,`slide_rels` 是其关系条目。
fn deck(slide: &str, slide_rels: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut parts = vec![
        ("ppt/presentation.xml", presentation(&["rId1"])),
        (
            "ppt/_rels/presentation.xml.rels",
            rels(&[("rId1", "slide", "slides/slide1.xml")]),
        ),
        (SLIDE, slide.to_string()),
    ];
    if !slide_rels.is_empty() {
        parts.push(("ppt/slides/_rels/slide1.xml.rels", rels(slide_rels)));
    }
    zip(&parts)
}

fn parse(bytes: &[u8]) -> ParsedPptx {
    parse_bytes(bytes).expect("parse")
}

fn kinds(p: &ParsedPptx, kind: DiagnosticKind) -> Vec<&Diagnostic> {
    p.presentation
        .diagnostics
        .iter()
        .filter(|d| d.kind == kind)
        .collect()
}

fn slide_text(p: &ParsedPptx) -> String {
    format!("{:?}", p.presentation.slides[0].shapes)
}

#[test]
fn clean_package_has_no_diagnostics() {
    let p = parse(&deck(&slide_xml(&text_sp("hello")), &[]));
    assert!(
        p.presentation.diagnostics.is_empty(),
        "{:?}",
        p.presentation.diagnostics
    );
}

#[test]
fn truncated_slide_xml_is_reported_and_the_parsed_prefix_is_kept() {
    // 第一个形状完整,第二个在属性中途被截断(读取错误)。
    let full = slide_xml(&format!("{}{}", text_sp("KEPT-PART"), text_sp(SECRET)));
    let cut = &full[..full.find(SECRET).unwrap() - 40];
    let p = parse(&deck(cut, &[]));
    assert!(slide_text(&p).contains("KEPT-PART"));
    let d = kinds(&p, DiagnosticKind::XmlTruncated);
    assert_eq!(d.len(), 1, "{:?}", p.presentation.diagnostics);
    assert_eq!(d[0].part, SLIDE);
    assert!(d[0].count > 0);
}

#[test]
fn xml_cut_after_a_complete_element_is_also_truncation() {
    // 没有读取错误,但 EOF 时标签未闭合:同样是内容被截断。
    let full = slide_xml(&format!("{}{}", text_sp("KEPT-PART"), text_sp(SECRET)));
    let cut = &full[..full.find("</p:sp>").unwrap() + "</p:sp>".len()];
    let p = parse(&deck(cut, &[]));
    assert!(slide_text(&p).contains("KEPT-PART"));
    assert_eq!(kinds(&p, DiagnosticKind::XmlTruncated).len(), 1);
}

#[test]
fn other_parts_are_covered_by_the_same_entry_point() {
    let master = format!(r#"<p:sldMaster {NS}><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="#);
    let mut parts = vec![
        ("ppt/presentation.xml", presentation(&["rId1"])),
        (
            "ppt/_rels/presentation.xml.rels",
            rels(&[("rId1", "slide", "slides/slide1.xml")]),
        ),
        (SLIDE, slide_xml(&text_sp("ok"))),
        (
            "ppt/slides/_rels/slide1.xml.rels",
            rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml")]),
        ),
        (
            "ppt/slideLayouts/slideLayout1.xml",
            format!(r#"<p:sldLayout {NS}><p:cSld><p:spTree/></p:cSld></p:sldLayout>"#),
        ),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]),
        ),
        ("ppt/slideMasters/slideMaster1.xml", master),
    ];
    parts.push(("ppt/theme/theme1.xml", "<a:theme".to_string()));
    let p = parse(&zip(&parts));
    let d = kinds(&p, DiagnosticKind::XmlTruncated);
    assert!(
        d.iter()
            .any(|d| d.part == "ppt/slideMasters/slideMaster1.xml"),
        "{d:?}"
    );
}

#[test]
fn nesting_beyond_the_limit_is_reported_with_a_count() {
    let depth = 70;
    let mut inner = text_sp("deep");
    for _ in 0..depth {
        inner = format!(
            r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="9" name="G"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{inner}</p:grpSp>"#
        );
    }
    let p = parse(&deck(&slide_xml(&inner), &[]));
    let d = kinds(&p, DiagnosticKind::NestingTooDeep);
    assert_eq!(d.len(), 1, "{:?}", p.presentation.diagnostics);
    assert_eq!((d[0].part.as_str(), d[0].count), (SLIDE, 1));
}

/// 公式结构嵌套超过上限时,更深的结构退化成纯拼接(文字不丢),并记一条 `nesting-too-deep`
/// 诊断(此前完全没有记录);浅公式不产生诊断。
#[test]
fn math_nesting_beyond_the_limit_is_reported_and_the_text_is_kept() {
    let math_sp = |inner: &str| {
        format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="M"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/>
<p:txBody><a:bodyPr/><a:p><a14:m xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:oMathPara><m:oMath>{inner}</m:oMath></m:oMathPara></a14:m></a:p></p:txBody></p:sp>"#
        )
    };
    let nest = |depth: usize| {
        let mut inner = "<m:r><m:t>DEEPTEXT</m:t></m:r>".to_string();
        for _ in 0..depth {
            inner = format!(
                "<m:sSup><m:e>{inner}</m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup>"
            );
        }
        inner
    };
    let p = parse(&deck(&slide_xml(&math_sp(&nest(70))), &[]));
    let d = kinds(&p, DiagnosticKind::NestingTooDeep);
    assert_eq!(d.len(), 1, "{:?}", p.presentation.diagnostics);
    assert_eq!(d[0].part, SLIDE);
    assert!(d[0].count >= 1);
    assert!(slide_text(&p).contains("DEEPTEXT"), "text must survive");

    let p = parse(&deck(&slide_xml(&math_sp(&nest(5))), &[]));
    assert!(kinds(&p, DiagnosticKind::NestingTooDeep).is_empty());
}

#[test]
fn duplicate_slide_references_are_reported_per_slide_part() {
    let bytes = zip(&[
        (
            "ppt/presentation.xml",
            presentation(&["rId1", "rId1", "rId2", "rId1"]),
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            rels(&[
                ("rId1", "slide", "slides/slide1.xml"),
                ("rId2", "slide", "slides/slide2.xml"),
            ]),
        ),
        (SLIDE, slide_xml(&text_sp("one"))),
        ("ppt/slides/slide2.xml", slide_xml(&text_sp("two"))),
    ]);
    let p = parse(&bytes);
    assert_eq!(p.presentation.slides.len(), 2);
    let d = kinds(&p, DiagnosticKind::DuplicateSlideRef);
    assert_eq!(d.len(), 1, "{:?}", p.presentation.diagnostics);
    assert_eq!((d[0].part.as_str(), d[0].count), (SLIDE, 2));
}

#[test]
fn relationships_to_missing_parts_are_counted_per_source_part() {
    let rels_in = [
        ("rId1", "image", "../media/gone1.png"),
        ("rId2", "image", "../media/gone2.png"),
        ("rId3", "notesSlide", "../notesSlides/notesSlide1.xml"),
    ];
    // 外链(超链接)不是包内部件,不算缺失。
    let mut slide_rels = rels(&rels_in);
    slide_rels = slide_rels.replace(
        "</Relationships>",
        r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/" TargetMode="External"/></Relationships>"#,
    );
    let bytes = zip(&[
        ("ppt/presentation.xml", presentation(&["rId1", "rId7"])),
        (
            "ppt/_rels/presentation.xml.rels",
            rels(&[
                ("rId1", "slide", "slides/slide1.xml"),
                ("rId7", "slide", "slides/slide404.xml"),
            ]),
        ),
        (SLIDE, slide_xml(&text_sp("one"))),
        ("ppt/slides/_rels/slide1.xml.rels", slide_rels),
    ]);
    let p = parse(&bytes);
    let mut d: Vec<(String, usize)> = kinds(&p, DiagnosticKind::MissingPart)
        .iter()
        .map(|d| (d.part.clone(), d.count))
        .collect();
    d.sort();
    assert_eq!(
        d,
        vec![
            ("ppt/presentation.xml".to_string(), 1),
            (SLIDE.to_string(), 3)
        ]
    );
}

fn graphic_frame(uri: &str, inner: &str) -> String {
    format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="F"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
<p:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></p:xfrm>
<a:graphic><a:graphicData uri="{uri}">{inner}</a:graphicData></a:graphic></p:graphicFrame>"#
    )
}

#[test]
fn smartart_without_a_usable_drawing_is_reported_as_degraded() {
    let dgm = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
    let frame = graphic_frame(
        dgm,
        &format!(
            r#"<dgm:relIds xmlns:dgm="{dgm}" r:dm="rId2" r:lo="rId3" r:qs="rId4" r:cs="rId5"/>"#
        ),
    );
    let data = format!(
        r#"<dgm:dataModel xmlns:dgm="{dgm}" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dgm:ptLst><dgm:pt modelId="1"><dgm:t><a:bodyPr/><a:p><a:r><a:t>{SECRET}</a:t></a:r></a:p></dgm:t></dgm:pt></dgm:ptLst></dgm:dataModel>"#
    );
    let bytes = zip(&[
        ("ppt/presentation.xml", presentation(&["rId1"])),
        (
            "ppt/_rels/presentation.xml.rels",
            rels(&[("rId1", "slide", "slides/slide1.xml")]),
        ),
        (SLIDE, slide_xml(&frame)),
        (
            "ppt/slides/_rels/slide1.xml.rels",
            rels(&[("rId2", "diagramData", "../diagrams/data1.xml")]),
        ),
        ("ppt/diagrams/data1.xml", data),
    ]);
    let p = parse(&bytes);
    let d = kinds(&p, DiagnosticKind::SmartArtDegraded);
    assert_eq!(d.len(), 1, "{:?}", p.presentation.diagnostics);
    assert_eq!(
        (d[0].part.as_str(), d[0].count),
        ("ppt/diagrams/data1.xml", 1)
    );
}

#[test]
fn malformed_chart_part_is_reported_as_degraded() {
    let uri = "http://schemas.openxmlformats.org/drawingml/2006/chart";
    let frame = graphic_frame(
        uri,
        r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId2"/>"#,
    );
    let bytes = zip(&[
        ("ppt/presentation.xml", presentation(&["rId1"])),
        (
            "ppt/_rels/presentation.xml.rels",
            rels(&[("rId1", "slide", "slides/slide1.xml")]),
        ),
        (SLIDE, slide_xml(&frame)),
        (
            "ppt/slides/_rels/slide1.xml.rels",
            rels(&[("rId2", "chart", "../charts/chart1.xml")]),
        ),
        (
            "ppt/charts/chart1.xml",
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:ser><c:tx>"#
                .to_string(),
        ),
    ]);
    let p = parse(&bytes);
    let d = kinds(&p, DiagnosticKind::ChartDegraded);
    assert_eq!(d.len(), 1, "{:?}", p.presentation.diagnostics);
    assert_eq!(
        (d[0].part.as_str(), d[0].count),
        ("ppt/charts/chart1.xml", 1)
    );
    assert_eq!(
        kinds(&p, DiagnosticKind::XmlTruncated)[0].part,
        "ppt/charts/chart1.xml"
    );
}

#[test]
fn diagnostics_never_carry_document_text() {
    // 截断 + 嵌套 + 缺失部件 + SmartArt 一起上,正文哨兵不得出现在任何诊断里。
    let full = slide_xml(&format!("{}{}", text_sp(SECRET), text_sp(SECRET)));
    let cut = &full[..full.rfind(SECRET).unwrap()];
    let p = parse(&deck(cut, &[("rId1", "image", "../media/gone.png")]));
    assert!(!p.presentation.diagnostics.is_empty());
    let dump = format!("{:?}", p.presentation.diagnostics);
    assert!(!dump.contains(SECRET), "{dump}");
    for d in &p.presentation.diagnostics {
        assert!(d.part.starts_with("ppt/"), "{d:?}");
        assert!(!d.kind.code().is_empty());
    }
}

#[test]
fn codes_are_stable_kebab_case() {
    use DiagnosticKind::*;
    let got: Vec<&str> = [
        XmlTruncated,
        NestingTooDeep,
        DuplicateSlideRef,
        MissingPart,
        SmartArtDegraded,
        ChartDegraded,
        CustomGeometryDegraded,
        DuplicateCommentRef,
        CommentsTruncated,
        ShapesTruncated,
        ContentTruncated,
    ]
    .iter()
    .map(|k| k.code())
    .collect();
    assert_eq!(
        got,
        [
            "xml-truncated",
            "nesting-too-deep",
            "duplicate-slide-ref",
            "missing-part",
            "smartart-degraded",
            "chart-degraded",
            "custom-geometry-degraded",
            "duplicate-comment-ref",
            "comments-truncated",
            "shapes-truncated",
            "content-truncated"
        ]
    );
}

#[test]
fn over_budget_custgeom_is_reported_with_part_and_count_only() {
    let guides: String = (0..=ppt_core::custgeom::MAX_GUIDES)
        .map(|i| format!(r#"<a:gd name="g{i}" fmla="val 1"/>"#))
        .collect();
    let sp = |secret: &str| {
        format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="{secret}"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
<p:spPr><a:custGeom><a:gdLst>{guides}</a:gdLst><a:pathLst/></a:custGeom>
<a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></p:spPr></p:sp>"#
        )
    };
    let p = parse(&deck(
        &slide_xml(&format!("{}{}", sp(SECRET), sp("x"))),
        &[],
    ));
    let d = kinds(&p, DiagnosticKind::CustomGeometryDegraded);
    assert_eq!(d.len(), 1, "{:?}", p.presentation.diagnostics);
    assert_eq!((d[0].part.as_str(), d[0].count), (SLIDE, 2));
    assert!(!format!("{:?}", p.presentation.diagnostics).contains(SECRET));
    // 预算内的 custGeom 不产生诊断。
    let ok = r#"<p:sp><p:spPr><a:custGeom><a:pathLst><a:path><a:moveTo><a:pt x="0" y="0"/></a:moveTo></a:path></a:pathLst></a:custGeom></p:spPr></p:sp>"#;
    let p = parse(&deck(&slide_xml(ok), &[]));
    assert!(kinds(&p, DiagnosticKind::CustomGeometryDegraded).is_empty());
}

// --- 诊断条数 / part 字段有界,且不泄露文件作者写的任意 Target 串 ---

const DGM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";

/// N 个 SmartArt frame,各指向一个不同名的不存在 data 部件。
fn missing_smartart_deck(n: usize, prefix: &str) -> Vec<u8> {
    let frames: String = (0..n)
        .map(|i| {
            graphic_frame(
                DGM,
                &format!(r#"<dgm:relIds xmlns:dgm="{DGM}" r:dm="rId{i}"/>"#),
            )
        })
        .collect();
    let slide_rels: Vec<(String, String)> = (0..n)
        .map(|i| (format!("rId{i}"), format!("../diagrams/{prefix}{i}.xml")))
        .collect();
    let refs: Vec<(&str, &str, &str)> = slide_rels
        .iter()
        .map(|(id, t)| (id.as_str(), "diagramData", t.as_str()))
        .collect();
    deck(&slide_xml(&frames), &refs)
}

#[test]
fn many_distinct_missing_targets_do_not_multiply_diagnostics() {
    let n = 10_000;
    let p = parse(&missing_smartart_deck(n, "ATTACKER-CHOSEN-"));
    let diags = &p.presentation.diagnostics;
    assert!(diags.len() <= 4, "条数必须与目标数无关:{}", diags.len());
    let total = |k: DiagnosticKind| -> usize {
        diags.iter().filter(|d| d.kind == k).map(|d| d.count).sum()
    };
    assert_eq!(total(DiagnosticKind::SmartArtDegraded), n, "总计数不丢");
    assert_eq!(total(DiagnosticKind::MissingPart), n);
    // 指向不存在部件的引用,part 记持有该关系的源部件,而不是文件里写的目标串。
    assert!(
        diags.iter().all(|d| d.part == SLIDE),
        "{:?}",
        diags.iter().map(|d| &d.part).collect::<Vec<_>>()
    );
    assert!(!format!("{diags:?}").contains("ATTACKER"));
}

#[test]
fn nesting_overflow_inside_comments_and_smartart_data_is_reported() {
    let nest = |depth: usize| {
        let mut inner = "<m:r><m:t>DEEPTEXT</m:t></m:r>".to_string();
        for _ in 0..depth {
            inner = format!(
                "<m:sSup><m:e>{inner}</m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup>"
            );
        }
        format!(
            r#"<a14:m xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:oMathPara><m:oMath>{inner}</m:oMath></m:oMathPara></a14:m>"#
        )
    };
    let comments = format!(
        r#"<p188:cmLst xmlns:p188="urn:p188" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><p188:cm><p188:txBody><a:bodyPr/><a:p>{}</a:p></p188:txBody></p188:cm></p188:cmLst>"#,
        nest(70)
    );
    let data = format!(
        r#"<dgm:dataModel xmlns:dgm="{DGM}" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dgm:ptLst><dgm:pt modelId="1"><dgm:t><a:bodyPr/><a:p>{}</a:p></dgm:t></dgm:pt></dgm:ptLst></dgm:dataModel>"#,
        nest(70)
    );
    let frame = graphic_frame(
        DGM,
        &format!(r#"<dgm:relIds xmlns:dgm="{DGM}" r:dm="rId2"/>"#),
    );
    let bytes = zip(&[
        ("ppt/presentation.xml", presentation(&["rId1"])),
        (
            "ppt/_rels/presentation.xml.rels",
            rels(&[("rId1", "slide", "slides/slide1.xml")]),
        ),
        (SLIDE, slide_xml(&frame)),
        (
            "ppt/slides/_rels/slide1.xml.rels",
            rels(&[
                ("rId2", "diagramData", "../diagrams/data1.xml"),
                ("rId3", "comments", "../comments/comment1.xml"),
            ]),
        ),
        ("ppt/diagrams/data1.xml", data),
        ("ppt/comments/comment1.xml", comments),
    ]);
    let p = parse(&bytes);
    let mut d: Vec<(String, usize)> = kinds(&p, DiagnosticKind::NestingTooDeep)
        .iter()
        .map(|d| (d.part.clone(), d.count))
        .collect();
    d.sort();
    let parts: Vec<&str> = d.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        parts,
        ["ppt/comments/comment1.xml", "ppt/diagrams/data1.xml"],
        "{:?}",
        p.presentation.diagnostics
    );
    assert!(d.iter().all(|(_, c)| *c >= 1));
}
