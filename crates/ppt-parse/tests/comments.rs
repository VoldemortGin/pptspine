//! 批注验收:旧式 `ppt/comments/commentN.xml`(`p:cmLst`,作者在 `commentAuthors.xml`)与新式
//! 线程批注(`p188:cmLst` + 回复,作者在 `authors.xml`),都经幻灯片 rels 定位。批注是审阅
//! 元数据:默认不进 `to_text` / Markdown。pptx 现场合成。

use std::io::{Cursor, Write};

use ppt_core::export::{presentation_markdown_with, presentation_text_with, ExportOptions};
use ppt_core::model::Comment;
use ppt_core::DiagnosticKind;
use ppt_parse::{parse_bytes, parse_bytes_with_limits, resolve, ZipLimits};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL_MS: &str = "http://schemas.microsoft.com/office/2018/10/relationships";
const P188: &str = "http://schemas.microsoft.com/office/powerpoint/2018/8/main";

fn rels(entries: &[(&str, String, &str)]) -> String {
    let body: String = entries
        .iter()
        .map(|(id, ty, t)| format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{t}"/>"#))
        .collect();
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
    )
}

fn deck(
    slide_rels: &[(&str, String, &str)],
    pres_extra_rels: &[(&str, String, &str)],
    extra: &[(&str, &str)],
) -> Vec<u8> {
    let mut pres_rels = vec![("rId1", format!("{REL}/slide"), "slides/slide1.xml")];
    pres_rels.extend(pres_extra_rels.iter().cloned());
    let mut parts: Vec<(String, String)> = vec![
        (
            "ppt/presentation.xml".into(),
            format!(
                r#"<p:presentation {NS}><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
            ),
        ),
        ("ppt/_rels/presentation.xml.rels".into(), rels(&pres_rels)),
        (
            "ppt/slides/slide1.xml".into(),
            format!(
                r#"<p:sld {NS}><p:cSld><p:spTree><p:sp><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>Body text</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
            ),
        ),
        ("ppt/slides/_rels/slide1.xml.rels".into(), rels(slide_rels)),
    ];
    parts.extend(extra.iter().map(|(n, b)| (n.to_string(), b.to_string())));
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

fn comments_of(bytes: &[u8]) -> Vec<Comment> {
    parse_bytes(bytes).expect("parse").presentation.slides[0]
        .comments
        .clone()
}

const LEGACY_AUTHORS: &str = r#"<p:cmAuthorLst xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
<p:cmAuthor id="0" name="Alice A" initials="AA" lastIdx="2" clrIdx="0"/><p:cmAuthor id="1" name="Bob" lastIdx="1" clrIdx="1"/></p:cmAuthorLst>"#;

const LEGACY: &str = r#"<p:cmLst xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
<p:cm authorId="0" dt="2024-05-06T10:11:12.000" idx="1"><p:pos x="10" y="20"/><p:text>First note</p:text></p:cm>
<p:cm authorId="1" dt="2024-05-07T08:00:00.000" idx="1"><p:pos x="-3" y="4"/><p:text>Second &amp; last</p:text></p:cm>
<p:cm authorId="9" idx="2"><p:text>Unknown author</p:text></p:cm></p:cmLst>"#;

fn legacy_deck() -> Vec<u8> {
    deck(
        &[(
            "rId5",
            format!("{REL}/comments"),
            "../comments/comment1.xml",
        )],
        &[(
            "rId9",
            format!("{REL}/commentAuthors"),
            "commentAuthors.xml",
        )],
        &[
            ("ppt/comments/comment1.xml", LEGACY),
            ("ppt/commentAuthors.xml", LEGACY_AUTHORS),
        ],
    )
}

#[test]
fn legacy_comments_resolve_authors_time_position_text() {
    let cs = comments_of(&legacy_deck());
    assert_eq!(cs.len(), 3);
    assert_eq!(cs[0].author.as_deref(), Some("Alice A"));
    assert_eq!(cs[0].initials.as_deref(), Some("AA"));
    assert_eq!(cs[0].datetime.as_deref(), Some("2024-05-06T10:11:12.000"));
    assert_eq!(cs[0].text.as_deref(), Some("First note"));
    assert_eq!(cs[0].position, Some((10, 20)));
    assert!(cs[0].replies.is_empty());
    // 作者无缩写 → None;位置可为负。
    assert_eq!(cs[1].author.as_deref(), Some("Bob"));
    assert_eq!(cs[1].initials, None);
    assert_eq!(cs[1].text.as_deref(), Some("Second & last"));
    assert_eq!(cs[1].position, Some((-3, 4)));
    // authorId 查不到、无 dt、无 pos → 全 None,正文仍在。
    assert_eq!(cs[2].author, None);
    assert_eq!(cs[2].datetime, None);
    assert_eq!(cs[2].position, None);
    assert_eq!(cs[2].text.as_deref(), Some("Unknown author"));
}

#[test]
fn modern_threaded_comments_with_replies() {
    let authors = format!(
        r#"<p188:authorLst xmlns:p188="{P188}"><p188:author id="{{G1}}" name="Carol" initials="C" userId="c@x" providerId="AD"/>
<p188:author id="{{G2}}" name="Dave" initials="D" userId="d@x" providerId="AD"/></p188:authorLst>"#
    );
    let cm = format!(
        r#"<p188:cmLst xmlns:p188="{P188}" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
<p188:cm id="{{C1}}" authorId="{{G1}}" created="2024-06-01T09:00:00.000">
  <p188:replyLst>
    <p188:reply id="{{R1}}" authorId="{{G2}}" created="2024-06-01T10:00:00.000"><p188:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Agreed</a:t></a:r></a:p></p188:txBody></p188:reply>
    <p188:reply id="{{R2}}" authorId="{{GX}}"><p188:txBody><a:bodyPr/><a:p><a:r><a:t>Orphan reply</a:t></a:r></a:p></p188:txBody></p188:reply>
  </p188:replyLst>
  <p188:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Line one</a:t></a:r></a:p><a:p><a:r><a:t>Line two</a:t></a:r></a:p></p188:txBody>
</p188:cm></p188:cmLst>"#
    );
    let bytes = deck(
        &[(
            "rId5",
            format!("{REL_MS}/comments"),
            "../comments/modernComment_1.xml",
        )],
        &[("rId9", format!("{REL_MS}/authors"), "authors.xml")],
        &[
            ("ppt/comments/modernComment_1.xml", &cm),
            ("ppt/authors.xml", &authors),
        ],
    );
    let cs = comments_of(&bytes);
    assert_eq!(cs.len(), 1);
    let c = &cs[0];
    assert_eq!(c.author.as_deref(), Some("Carol"));
    assert_eq!(c.initials.as_deref(), Some("C"));
    assert_eq!(c.datetime.as_deref(), Some("2024-06-01T09:00:00.000"));
    assert_eq!(c.text.as_deref(), Some("Line one\nLine two"));
    assert_eq!(c.position, None);
    assert_eq!(c.replies.len(), 2);
    assert_eq!(c.replies[0].author.as_deref(), Some("Dave"));
    assert_eq!(
        c.replies[0].datetime.as_deref(),
        Some("2024-06-01T10:00:00.000")
    );
    assert_eq!(c.replies[0].text.as_deref(), Some("Agreed"));
    assert_eq!(c.replies[1].author, None);
    assert_eq!(c.replies[1].text.as_deref(), Some("Orphan reply"));
}

#[test]
fn missing_authors_part_leaves_author_none() {
    let bytes = deck(
        &[(
            "rId5",
            format!("{REL}/comments"),
            "../comments/comment1.xml",
        )],
        &[],
        &[("ppt/comments/comment1.xml", LEGACY)],
    );
    let cs = comments_of(&bytes);
    assert_eq!(cs.len(), 3);
    assert!(cs
        .iter()
        .all(|c| c.author.is_none() && c.initials.is_none()));
    assert_eq!(cs[0].text.as_deref(), Some("First note"));
}

#[test]
fn missing_or_malformed_parts_do_not_panic() {
    // 关系指向不存在的部件。
    let bytes = deck(
        &[("rId5", format!("{REL}/comments"), "../comments/none.xml")],
        &[],
        &[],
    );
    assert!(comments_of(&bytes).is_empty());
    // 部件不是 XML / 截断 / 作者部件畸形。
    for bad in [
        "garbage <<<",
        "<p:cmLst xmlns:p=\"urn:p\"><p:cm authorId=\"0\"><p:text>cut",
        "",
    ] {
        let bytes = deck(
            &[(
                "rId5",
                format!("{REL}/comments"),
                "../comments/comment1.xml",
            )],
            &[(
                "rId9",
                format!("{REL}/commentAuthors"),
                "commentAuthors.xml",
            )],
            &[
                ("ppt/comments/comment1.xml", bad),
                ("ppt/commentAuthors.xml", "<<<nope"),
            ],
        );
        let _ = comments_of(&bytes); // 只要不 panic;截断者可尽力保留已读部分。
    }
    // 无批注关系 → 空。
    assert!(comments_of(&deck(&[], &[], &[])).is_empty());
}

#[test]
fn default_exports_exclude_comment_text() {
    let bytes = legacy_deck();
    let parsed = parse_bytes(&bytes).unwrap();
    let resolved = resolve(&parsed);
    let opts = ExportOptions::default();
    let text = presentation_text_with(&parsed.presentation, Some(&resolved), &opts);
    let md = presentation_markdown_with(&parsed.presentation, Some(&resolved), &opts);
    for out in [&text, &md] {
        assert!(out.contains("Body text"));
        for secret in ["First note", "Second", "Alice", "Bob", "Unknown author"] {
            assert!(!out.contains(secret), "{secret} leaked into export: {out}");
        }
    }
}

// --- 放大攻击:同一批注部件被重复 / 共享引用,总条数必须有界 ---

/// `n_slides` 张幻灯片,每张的 rels 里有 `rels_per_slide` 条指向同一批注部件的 comments 关系。
fn shared_deck(n_slides: usize, rels_per_slide: usize, comment_part: &str) -> Vec<u8> {
    let mut parts: Vec<(String, String)> = Vec::new();
    let ids: String = (1..=n_slides)
        .map(|i| format!(r#"<p:sldId id="{}" r:id="rId{i}"/>"#, 255 + i))
        .collect();
    parts.push((
        "ppt/presentation.xml".into(),
        format!(
            r#"<p:presentation {NS}><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
        ),
    ));
    let pres_rels: Vec<(String, String, String)> = (1..=n_slides)
        .map(|i| {
            (
                format!("rId{i}"),
                format!("{REL}/slide"),
                format!("slides/slide{i}.xml"),
            )
        })
        .collect();
    let rel_xml = |items: &[(String, String, String)]| {
        let body: String = items
            .iter()
            .map(|(id, ty, t)| format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{t}"/>"#))
            .collect();
        format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
        )
    };
    parts.push((
        "ppt/_rels/presentation.xml.rels".into(),
        rel_xml(&pres_rels),
    ));
    let slide_rels: Vec<(String, String, String)> = (0..rels_per_slide)
        .map(|i| {
            (
                format!("rId{i}"),
                format!("{REL}/comments"),
                "../comments/comment1.xml".to_string(),
            )
        })
        .collect();
    for i in 1..=n_slides {
        parts.push((
            format!("ppt/slides/slide{i}.xml"),
            format!(r#"<p:sld {NS}><p:cSld><p:spTree/></p:cSld></p:sld>"#),
        ));
        parts.push((
            format!("ppt/slides/_rels/slide{i}.xml.rels"),
            rel_xml(&slide_rels),
        ));
    }
    parts.push(("ppt/comments/comment1.xml".into(), comment_part.to_string()));
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

fn diag(p: &ppt_parse::ParsedPptx, kind: DiagnosticKind) -> Vec<(String, usize)> {
    p.presentation
        .diagnostics
        .iter()
        .filter(|d| d.kind == kind)
        .map(|d| (d.part.clone(), d.count))
        .collect()
}

#[test]
fn same_part_referenced_by_50_rels_is_read_once_per_slide() {
    let p = parse_bytes(&shared_deck(1, 50, LEGACY)).unwrap();
    assert_eq!(p.presentation.slides[0].comments.len(), 3);
    assert_eq!(
        diag(&p, DiagnosticKind::DuplicateCommentRef),
        [("ppt/comments/comment1.xml".to_string(), 49)]
    );
}

#[test]
fn part_shared_across_slides_is_cached_and_each_slide_still_gets_it() {
    let p = parse_bytes(&shared_deck(3, 1, LEGACY)).unwrap();
    for s in &p.presentation.slides {
        assert_eq!(s.comments.len(), 3);
        assert_eq!(s.comments, p.presentation.slides[0].comments);
    }
    assert!(diag(&p, DiagnosticKind::DuplicateCommentRef).is_empty());
    assert!(diag(&p, DiagnosticKind::CommentsTruncated).is_empty());
}

#[test]
fn total_comment_budget_truncates_across_slides_and_records_diagnostic() {
    // 3 张幻灯片 × 3 条 = 9;预算 7 => 3 + 3 + 1。
    let limits = ZipLimits {
        max_comments: 7,
        ..ZipLimits::default()
    };
    let p = parse_bytes_with_limits(&shared_deck(3, 1, LEGACY), &limits).unwrap();
    let counts: Vec<usize> = p
        .presentation
        .slides
        .iter()
        .map(|s| s.comments.len())
        .collect();
    assert_eq!(counts, [3, 3, 1]);
    assert_eq!(
        diag(&p, DiagnosticKind::CommentsTruncated),
        [("ppt/comments/comment1.xml".to_string(), 1)]
    );
}

#[test]
fn comment_budget_stops_parsing_a_huge_part_and_counts_replies() {
    // 单个部件 1 000 条,预算 10 => 只保留 10 条;回复也计数(1 条批注 + 2 条回复,预算 2)。
    let many = format!(
        r#"<p:cmLst xmlns:p="urn:p">{}</p:cmLst>"#,
        r#"<p:cm authorId="0"><p:text>x</p:text></p:cm>"#.repeat(1_000)
    );
    let limits = ZipLimits {
        max_comments: 10,
        ..ZipLimits::default()
    };
    let p = parse_bytes_with_limits(&shared_deck(1, 1, &many), &limits).unwrap();
    assert_eq!(p.presentation.slides[0].comments.len(), 10);
    assert_eq!(diag(&p, DiagnosticKind::CommentsTruncated).len(), 1);

    let threaded = format!(
        r#"<p188:cmLst xmlns:p188="{P188}"><p188:cm><p188:replyLst><p188:reply/><p188:reply/></p188:replyLst></p188:cm></p188:cmLst>"#
    );
    let limits = ZipLimits {
        max_comments: 2,
        ..ZipLimits::default()
    };
    let p = parse_bytes_with_limits(&shared_deck(1, 1, &threaded), &limits).unwrap();
    let c = &p.presentation.slides[0].comments;
    assert_eq!((c.len(), c[0].replies.len()), (1, 1));
    assert_eq!(diag(&p, DiagnosticKind::CommentsTruncated).len(), 1);
}

#[test]
fn defaults_for_comment_budget() {
    assert_eq!(ZipLimits::default().max_comments, 100_000);
}
