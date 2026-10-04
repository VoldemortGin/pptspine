//! "字节 × 引用次数"放大族:被多处引用的值必须**共享**(同一份字节,不按引用次数克隆),
//! 单个值必须有长度上限,超长截断并记 `value-truncated` 诊断。每个用例是一个生成器
//! (规模参数可调),这里只跑小规模;断言共享看字符串数据指针是否相同。

use std::io::{Cursor, Write};

use ppt_core::model::{GraphicPlaceholder, Shape};
use ppt_parse::{parse_bytes, parse_bytes_with_limits, ParsedPptx, ZipLimits};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const CHART: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const SLIDE1: &str = "ppt/slides/slide1.xml";

fn rels(items: &[(String, String, String)]) -> String {
    let body: String = items
        .iter()
        .map(|(id, ty, t)| format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{t}"/>"#))
        .collect();
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
    )
}

fn rel(id: &str, ty: &str, target: &str) -> (String, String, String) {
    (id.to_string(), ty.to_string(), target.to_string())
}

/// 单张幻灯片(spTree 内容 + rels)+ 额外部件。
fn build(tree: &str, srels: &[(String, String, String)], extra: &[(&str, String)]) -> Vec<u8> {
    let mut parts: Vec<(String, String)> = vec![
        (
            "ppt/presentation.xml".into(),
            format!(
                r#"<p:presentation {NS}><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
            ),
        ),
        (
            "ppt/_rels/presentation.xml.rels".into(),
            rels(&[rel("rId1", &format!("{REL}/slide"), "slides/slide1.xml")]),
        ),
        (
            SLIDE1.into(),
            format!(r#"<p:sld {NS}><p:cSld><p:spTree>{tree}</p:spTree></p:cSld></p:sld>"#),
        ),
        ("ppt/slides/_rels/slide1.xml.rels".into(), rels(srels)),
    ];
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

/// `part` 上某种诊断(按稳定 code)的总计数。
fn diag(p: &ParsedPptx, code: &str, part: &str) -> usize {
    p.presentation
        .diagnostics
        .iter()
        .filter(|d| d.kind.code() == code && d.part == part)
        .map(|d| d.count)
        .sum()
}

fn text_box(inner: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/>{inner}</p:txBody></p:sp>"#
    )
}

/// 生成器:一个 `name_len` 字节的作者名 × `n` 条批注。
fn author_deck(name_len: usize, n: usize) -> Vec<u8> {
    let authors = format!(
        r#"<p:cmAuthorLst xmlns:p="urn:p"><p:cmAuthor id="0" name="{}" initials="{}"/></p:cmAuthorLst>"#,
        "A".repeat(name_len),
        "I".repeat(name_len)
    );
    let cm = format!(
        r#"<p:cmLst xmlns:p="urn:p">{}</p:cmLst>"#,
        r#"<p:cm authorId="0"/>"#.repeat(n)
    );
    build(
        "",
        &[rel(
            "rId1",
            &format!("{REL}/comments"),
            "../comments/comment1.xml",
        )],
        &[
            ("ppt/comments/comment1.xml", cm),
            ("ppt/commentAuthors.xml", authors),
        ],
    )
}

/// 批注作者名被每条批注**共享**(同一份字节),且截到 4 KiB 标签上限并记诊断。
#[test]
fn comment_author_is_shared_across_comments_and_capped() {
    let p = parse_bytes(&author_deck(64 * 1024, 200)).unwrap();
    let cs = &p.presentation.slides[0].comments;
    assert_eq!(cs.len(), 200);
    let ptrs: Vec<_> = cs
        .iter()
        .map(|c| c.author.as_deref().map(str::as_ptr))
        .collect();
    assert!(ptrs[0].is_some());
    assert!(
        ptrs.iter().all(|x| *x == ptrs[0]),
        "作者名必须共享同一份字节"
    );
    let initials: Vec<_> = cs
        .iter()
        .map(|c| c.initials.as_deref().map(str::as_ptr))
        .collect();
    assert!(initials.iter().all(|x| *x == initials[0]));
    assert_eq!(cs[0].author.as_deref().map(str::len), Some(4 * 1024));
    assert_eq!(diag(&p, "value-truncated", "ppt/commentAuthors.xml"), 2);
}

/// 生成器:一个 `url_len` 字节的外链 × `n` 个 run。
fn link_deck(url_len: usize, n: usize) -> Vec<u8> {
    let run = r#"<a:r><a:rPr><a:hlinkClick r:id="rId7"/></a:rPr><a:t>x</a:t></a:r>"#;
    let url = format!("http://e.com/{}", "u".repeat(url_len));
    build(
        &text_box(&format!("<a:p>{}</a:p>", run.repeat(n))),
        &[(
            "rId7".into(),
            format!("{REL}/hyperlink"),
            format!(r#"{url}" TargetMode="External"#),
        )],
        &[],
    )
}

fn run_urls(p: &ParsedPptx) -> Vec<Option<(*const u8, usize)>> {
    let Shape::TextBox(t) = &p.presentation.slides[0].shapes[0] else {
        panic!("expected a text box");
    };
    t.paragraphs[0]
        .runs
        .iter()
        .map(|r| {
            r.hyperlink
                .as_ref()
                .and_then(|h| h.url.as_deref())
                .map(|u| (u.as_ptr(), u.len()))
        })
        .collect()
}

/// 超链接目标被引用同一关系的所有 run **共享**,且截到 16 KiB 并记诊断。
#[test]
fn hyperlink_target_is_shared_across_runs_and_capped() {
    let p = parse_bytes(&link_deck(1_000, 300)).unwrap();
    let urls = run_urls(&p);
    assert_eq!(urls.len(), 300);
    assert!(urls[0].is_some());
    assert!(urls.iter().all(|u| *u == urls[0]), "URL 必须共享同一份字节");
    assert_eq!(diag(&p, "value-truncated", SLIDE1), 0);

    let long = parse_bytes(&link_deck(40 * 1024, 3)).unwrap();
    let urls = run_urls(&long);
    assert_eq!(urls[0].map(|u| u.1), Some(16 * 1024));
    assert_eq!(diag(&long, "value-truncated", SLIDE1), 1);
}

/// 生成器:一个类别名 `cat_len` 字节的图表 × `frames` 个 frame。
fn chart_deck(cat_len: usize, frames: usize) -> Vec<u8> {
    let frame = format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="F"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
<p:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></p:xfrm>
<a:graphic><a:graphicData uri="{CHART}"><c:chart xmlns:c="{CHART}" r:id="rId9"/></a:graphicData></a:graphic></p:graphicFrame>"#
    );
    let chart = format!(
        r#"<c:chartSpace xmlns:c="{CHART}"><c:chart><c:plotArea><c:barChart><c:ser><c:tx><c:v>{}</c:v></c:tx>
<c:cat><c:strLit><c:ptCount val="1"/><c:pt idx="0"><c:v>{}</c:v></c:pt></c:strLit></c:cat>
<c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#,
        "S".repeat(cat_len),
        "K".repeat(cat_len)
    );
    build(
        &frame.repeat(frames),
        &[rel("rId9", &format!("{REL}/chart"), "../charts/chart1.xml")],
        &[("ppt/charts/chart1.xml", chart)],
    )
}

/// 同一图表部件被多个 frame 引用:各 frame **共享**同一份数据(类别名 / 系列名不逐 frame 复制),
/// 且类别名 / 系列名截到 4 KiB 标签上限并记诊断。
#[test]
fn chart_data_is_shared_across_frames_and_labels_are_capped() {
    let p = parse_bytes(&chart_deck(8 * 1024, 40)).unwrap();
    let cats: Vec<_> = p.presentation.slides[0]
        .shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Placeholder(GraphicPlaceholder { chart: Some(c), .. }) => {
                Some((c.categories[0].as_ptr(), c.categories[0].len()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(cats.len(), 40);
    assert!(cats.iter().all(|c| *c == cats[0]), "图表数据必须共享");
    assert_eq!(cats[0].1, 4 * 1024);
    assert_eq!(diag(&p, "value-truncated", "ppt/charts/chart1.xml"), 2);
}

/// 生成器:`m:sepChr` 长 `sep_len` 字节 × `n` 个 `m:e`。
fn math_sep_deck(sep_len: usize, n: usize) -> Vec<u8> {
    let omath = format!(
        r#"<m:d><m:dPr><m:sepChr m:val="{}"/><m:begChr m:val="{}"/></m:dPr>{}</m:d>"#,
        "S".repeat(sep_len),
        "B".repeat(sep_len),
        "<m:e><m:r><m:t>x</m:t></m:r></m:e>".repeat(n)
    );
    let run = format!(
        r#"<a14:m xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:oMathPara><m:oMath>{omath}</m:oMath></m:oMathPara></a14:m>"#
    );
    build(&text_box(&format!("<a:p>{run}</a:p>")), &[], &[])
}

/// 公式定界符 / 分隔符规范上是单字符:截到 2 个字符,超长分隔符不会被每个 `m:e` 各拼一次。
#[test]
fn math_delimiters_are_capped_to_two_chars() {
    let p = parse_bytes(&math_sep_deck(4 * 1024, 100)).unwrap();
    let text = p.presentation.slides[0]
        .shapes
        .iter()
        .find_map(|s| match s {
            Shape::TextBox(t) => Some(t.paragraphs[0].runs[0].text.clone()),
            _ => None,
        });
    let text = text.expect("math run");
    // "BB" + 100 个 "x" + 99 个 "SS" + ")"。
    assert_eq!(text.len(), 2 + 100 + 99 * 2 + 1, "{}", &text[..40]);
    assert!(text.starts_with("BBx"));
    assert_eq!(diag(&p, "value-truncated", SLIDE1), 2);
}

/// 备注部件与其它部件同一套预算:调用方给的单部件节点上限对备注同样生效,截断记诊断。
#[test]
fn notes_obey_caller_limits_and_report_truncation() {
    let notes = format!(
        r#"<p:notes {NS}><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="2" name="n"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/>{}<a:p><a:r><a:t>NOTES_END</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#,
        "<a:p><a:r><a:t>a</a:t></a:r></a:p>".repeat(2_000)
    );
    let bytes = build(
        &text_box("<a:p><a:r><a:t>hi</a:t></a:r></a:p>"),
        &[rel(
            "rId3",
            &format!("{REL}/notesSlide"),
            "../notesSlides/notesSlide1.xml",
        )],
        &[("ppt/notesSlides/notesSlide1.xml", notes)],
    );
    let limits = ZipLimits {
        max_part_items: 500,
        ..ZipLimits::default()
    };
    let p = parse_bytes_with_limits(&bytes, &limits).unwrap();
    let n = p.presentation.slides[0].notes.as_deref().unwrap_or("");
    assert_eq!(n.lines().count(), 500, "备注段落数受调用方的单部件上限约束");
    assert!(!n.contains("NOTES_END"));
    assert_eq!(
        diag(&p, "content-truncated", "ppt/notesSlides/notesSlide1.xml"),
        1_501
    );
}
