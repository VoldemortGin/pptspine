//! 模糊测试 / 审查中发现并修掉的问题的**最小触发输入**回归(现场构造,不提交 crash 二进制):
//! 展开写法的叶子元素(`<a:off …></a:off>`)、同一幻灯片被重复引用、没有 `mc:Fallback` 的
//! `mc:AlternateContent`、深嵌套公式 / 组合 / AlternateContent。每个输入都走与 fuzz target 相同
//! 的链路(`parse_bytes` → `resolve` → 文本 / Markdown 导出),普通 `cargo test` 就能跑到。

use std::io::{Cursor, Write};

use ppt_core::model::{Fill, RunKind, Shape};
use ppt_core::{presentation_markdown_with, presentation_text_with, ExportOptions};
use ppt_parse::{parse_bytes, resolve, ParsedPptx};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"
       xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"
       xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main"
       xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math""#;
const SLIDE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";

fn pack(sld_id_lst: &str, slide_xml: &str) -> Vec<u8> {
    let parts = [
        (
            "ppt/presentation.xml",
            format!(
                r#"<p:presentation {NS}><p:sldIdLst>{sld_id_lst}</p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
            ),
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{SLIDE_REL}" Target="slides/slide1.xml"/></Relationships>"#
            ),
        ),
        ("ppt/slides/slide1.xml", slide_xml.to_string()),
    ];
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        for (name, body) in &parts {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buf.into_inner()
}

fn slide(sp_tree: &str) -> String {
    format!(r#"<p:sld {NS}><p:cSld><p:spTree>{sp_tree}</p:spTree></p:cSld></p:sld>"#)
}

/// fuzz target 同款链路:解析 → 终态 IR → 两个文本导出器;返回解析结果供断言。
fn run(sld_id_lst: &str, sp_tree: &str) -> ParsedPptx {
    let parsed = parse_bytes(&pack(sld_id_lst, &slide(sp_tree))).expect("parse");
    let resolved = resolve(&parsed);
    let opts = ExportOptions::default();
    let _ = presentation_text_with(&parsed.presentation, Some(&resolved), &opts);
    let _ = presentation_markdown_with(&parsed.presentation, Some(&resolved), &opts);
    parsed
}

fn run_one(sp_tree: &str) -> ParsedPptx {
    run(r#"<p:sldId id="256" r:id="rId1"/>"#, sp_tree)
}

fn text_of(shape: &Shape) -> String {
    match shape {
        Shape::TextBox(tf) => tf
            .paragraphs
            .iter()
            .flat_map(|p| &p.runs)
            .map(|r| r.text.as_str())
            .collect(),
        Shape::Auto(a) => a
            .text
            .iter()
            .flat_map(|tf| tf.paragraphs.iter().flat_map(|p| &p.runs))
            .map(|r| r.text.as_str())
            .collect(),
        _ => String::new(),
    }
}

/// 展开写法的 `a:off` / `a:ext` / `a:avLst`:`parse_xfrm` 曾在第一个子元素结束标签处返回,
/// 之后的填充 / 几何 / 描边整体丢失。
#[test]
fn expanded_form_leaf_elements_keep_everything_after_xfrm() {
    let parsed = run_one(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="S"></p:cNvPr><p:cNvSpPr></p:cNvSpPr><p:nvPr></p:nvPr></p:nvSpPr>
<p:spPr><a:xfrm><a:off x="5" y="6"></a:off><a:ext cx="7" cy="8"></a:ext></a:xfrm>
<a:prstGeom prst="ellipse"><a:avLst></a:avLst></a:prstGeom>
<a:solidFill><a:srgbClr val="FF0000"></a:srgbClr></a:solidFill></p:spPr></p:sp>"#,
    );
    let Shape::Auto(a) = &parsed.presentation.slides[0].shapes[0] else {
        panic!("{:?}", parsed.presentation.slides[0].shapes);
    };
    assert_eq!(a.rect.map(|r| (r.x, r.y, r.w, r.h)), Some((5, 6, 7, 8)));
    assert_eq!(a.geometry.as_deref(), Some("ellipse"));
    assert!(matches!(a.fill, Some(Fill::Solid(_))), "{:?}", a.fill);
}

/// 自闭合 `<a:p/>` 与展开的 `<a:p></a:p>` 都是一个空段落(曾前者被丢弃)。
#[test]
fn self_closed_and_expanded_empty_paragraphs_agree() {
    let paras = |p: &str| {
        let parsed = run_one(&format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="S"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/>{p}</p:txBody></p:sp>"#
        ));
        match &parsed.presentation.slides[0].shapes[0] {
            Shape::TextBox(tf) => tf.paragraphs.len(),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(paras("<a:p/>"), 1);
    assert_eq!(paras("<a:p></a:p>"), 1);
}

/// 同一幻灯片被 `p:sldIdLst` 重复引用:只解析 / 存储一份(曾可把一页放大成 N 份)。
#[test]
fn repeated_slide_references_are_deduplicated() {
    let refs = r#"<p:sldId id="256" r:id="rId1"/>"#.repeat(5_000);
    let parsed = run(&refs, "");
    assert_eq!(parsed.presentation.slides.len(), 1);
    // 去掉的 4999 次重复引用在诊断里留痕。
    let d = &parsed.presentation.diagnostics;
    assert!(d.iter().any(|d| d.count == 4_999), "{d:?}");
}

/// 没有 `mc:Fallback` 的 `mc:AlternateContent`:Choice 里认得的内容必须保留(曾整块丢失)。
#[test]
fn alternate_content_without_fallback_keeps_the_choice() {
    let sp = r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="S"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/>
<p:txBody><a:bodyPr/><a:p><a:r><a:t>CHOICE-ONLY</a:t></a:r></a:p></p:txBody></p:sp>"#;
    let parsed = run_one(&format!(
        r#"<mc:AlternateContent><mc:Choice Requires="p14">{sp}</mc:Choice></mc:AlternateContent>"#
    ));
    let shapes = &parsed.presentation.slides[0].shapes;
    assert_eq!(shapes.len(), 1, "{shapes:?}");
    assert!(text_of(&shapes[0]).contains("CHOICE-ONLY"));
    // 段落层同理:公式在 Choice、无 Fallback。
    let para = run_one(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="S"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p>
<mc:AlternateContent><mc:Choice Requires="a14"><a14:m><m:oMathPara><m:oMath><m:r><m:t>x+1</m:t></m:r></m:oMath></m:oMathPara></a14:m></mc:Choice></mc:AlternateContent>
</a:p></p:txBody></p:sp>"#,
    );
    let Shape::TextBox(tf) = &para.presentation.slides[0].shapes[0] else {
        panic!();
    };
    let run = &tf.paragraphs[0].runs[0];
    assert_eq!(
        (run.kind.clone(), run.text.as_str()),
        (RunKind::Math, "x+1")
    );
}

/// 深嵌套公式(迭代遍历,深度上限):数万层 `m:f > m:num` 不爆栈、文字不丢。
#[test]
fn deeply_nested_math_does_not_overflow_the_stack() {
    let n = 20_000;
    let math = format!(
        "<a14:m><m:oMathPara><m:oMath>{}<m:r><m:t>DEEP</m:t></m:r>{}</m:oMath></m:oMathPara></a14:m>",
        "<m:f><m:num>".repeat(n),
        "</m:num></m:f>".repeat(n)
    );
    let parsed = run_one(&format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="S"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p>{math}</a:p></p:txBody></p:sp>"#
    ));
    let Shape::TextBox(tf) = &parsed.presentation.slides[0].shapes[0] else {
        panic!();
    };
    assert!(tf.paragraphs[0].runs[0].text.contains("DEEP"));
}

/// 深嵌套组合 / AlternateContent:超过上限的子树整体跳过(不爆栈),并在诊断里留痕。
#[test]
fn deeply_nested_groups_and_alternate_content_are_bounded() {
    let n = 5_000;
    let groups = format!(
        r#"{}<p:sp><p:nvSpPr><p:cNvPr id="2" name="S"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>x</a:t></a:r></a:p></p:txBody></p:sp>{}"#,
        r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="9" name="G"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>"#.repeat(n),
        "</p:grpSp>".repeat(n)
    );
    let parsed = run_one(&groups);
    assert!(parsed
        .presentation
        .diagnostics
        .iter()
        .any(|d| d.kind.code() == "nesting-too-deep"));
    let alt = format!(
        "{}<p:sp><p:spPr/></p:sp>{}",
        r#"<mc:AlternateContent><mc:Choice Requires="x">"#.repeat(n),
        "</mc:Choice></mc:AlternateContent>".repeat(n)
    );
    let _ = run_one(&alt);
}
