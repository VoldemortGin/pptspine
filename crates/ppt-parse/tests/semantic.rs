//! 语义抽取解析验收:隐藏页(`p:sld@show`)、节(`p14:sectionLst`)、文档属性
//! (`docProps/core.xml` + `app.xml`)、超链接(run 级 / 形状级 `a:hlinkClick`,外链 +
//! 内部跳转)、图片替代文本(`p:cNvPr@descr/@title/@name`)。三张 slide 现场合成。

use std::io::{Cursor, Write};

use ppt_core::model::Shape;
use ppt_parse::parse_bytes;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;

const REL_SLIDE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";
const REL_LINK: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";

fn slide(attrs: &str, sp_tree: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld {NS}{attrs}><p:cSld><p:spTree>{sp_tree}</p:spTree></p:cSld></p:sld>"#
    )
}

fn rels(entries: &[(&str, &str, &str, bool)]) -> String {
    let body: String = entries
        .iter()
        .map(|(id, ty, target, ext)| {
            let mode = if *ext {
                r#" TargetMode="External""#
            } else {
                ""
            };
            format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{target}"{mode}/>"#)
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
    )
}

fn build(parts: &[(&str, String)]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default();
        for (name, body) in parts {
            zip.start_file(*name, opts).expect("start_file");
            zip.write_all(body.as_bytes()).expect("write");
        }
        zip.finish().expect("finish zip");
    }
    buf.into_inner()
}

fn deck() -> Vec<u8> {
    let presentation = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation {NS}>
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/><p:sldId id="257" r:id="rId2"/><p:sldId r:id="rId3" id="258"/></p:sldIdLst>
  <p:sldSz cx="9144000" cy="6858000"/>
  <p:extLst><p:ext uri="{{521415D9-36F7-43E2-AB2F-B90AF26B5E84}}">
    <p14:sectionLst xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main">
      <p14:section name="Intro" id="{{A}}"><p14:sldIdLst><p14:sldId id="256"/></p14:sldIdLst></p14:section>
      <p14:section name="Body &amp; End" id="{{B}}"><p14:sldIdLst><p14:sldId id="258"/><p14:sldId id="257"/><p14:sldId id="999"/></p14:sldIdLst></p14:section>
      <p14:section name="Empty" id="{{C}}"/>
    </p14:sectionLst>
  </p:ext></p:extLst>
</p:presentation>"#
    );
    let s1 = slide(
        "",
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Box"><a:hlinkClick r:id="rId9" tooltip="tip"/></p:cNvPr><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
  <p:spPr/><p:txBody><a:bodyPr/><a:p>
    <a:r><a:rPr lang="en-US"><a:hlinkClick r:id="rId7"/></a:rPr><a:t>site</a:t></a:r>
    <a:r><a:rPr><a:hlinkClick r:id="rId8" action="ppaction://hlinksldjump"/></a:rPr><a:t>jump</a:t></a:r>
    <a:r><a:rPr/><a:t> next</a:t></a:r>
    <a:r><a:rPr><a:latin typeface="Arial"/><a:hlinkClick r:id="" action="ppaction://hlinkshowjump?jump=nextslide"/></a:rPr><a:t>!</a:t></a:r>
  </a:p></p:txBody></p:sp>
<p:pic><p:nvPicPr><p:cNvPr id="3" name="Picture 2" descr="A red square" title="Logo"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
  <p:blipFill><a:blip r:embed="rId5"/></p:blipFill><p:spPr/></p:pic>"#,
    );
    let s2 = slide(r#" show="0""#, "");
    let s3 = slide(r#" show="1""#, "");
    let s1_rels = rels(&[
        ("rId7", REL_LINK, "https://example.com/a?b=1", true),
        ("rId8", REL_SLIDE, "slide3.xml", false),
        ("rId9", REL_LINK, "mailto:x@example.com", true),
    ]);
    let core = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
  xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/">
  <dc:title>Deck Title</dc:title><dc:creator>Ada</dc:creator>
  <dcterms:created>2026-01-02T03:04:05Z</dcterms:created>
</cp:coreProperties>"#;
    let app = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>PowerPoint</Application></Properties>"#;
    build(&[
        (
            "_rels/.rels",
            rels(&[
                ("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument", "ppt/presentation.xml", false),
                ("rId2", "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties", "docProps/core.xml", false),
                ("rId3", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties", "/docProps/app.xml", false),
            ]),
        ),
        ("docProps/core.xml", core.to_string()),
        ("docProps/app.xml", app.to_string()),
        ("ppt/presentation.xml", presentation),
        (
            "ppt/_rels/presentation.xml.rels",
            rels(&[
                ("rId1", REL_SLIDE, "slides/slide1.xml", false),
                ("rId2", REL_SLIDE, "slides/slide2.xml", false),
                ("rId3", REL_SLIDE, "slides/slide3.xml", false),
            ]),
        ),
        ("ppt/slides/slide1.xml", s1),
        ("ppt/slides/_rels/slide1.xml.rels", s1_rels),
        ("ppt/slides/slide2.xml", s2),
        ("ppt/slides/slide3.xml", s3),
    ])
}

#[test]
fn hidden_slides_from_show_attribute() {
    let p = parse_bytes(&deck()).expect("parse").presentation;
    let hidden: Vec<bool> = p.slides.iter().map(|s| s.hidden).collect();
    assert_eq!(hidden, vec![false, true, false]);
}

#[test]
fn sections_map_slide_ids_to_indices() {
    let p = parse_bytes(&deck()).expect("parse").presentation;
    let got: Vec<(&str, Vec<usize>)> = p
        .sections
        .iter()
        .map(|s| (s.name.as_str(), s.slide_indices.clone()))
        .collect();
    // 未知 id(999)丢弃;节内顺序保留;自闭合空节保留为空。
    assert_eq!(
        got,
        vec![
            ("Intro", vec![0]),
            ("Body & End", vec![2, 1]),
            ("Empty", vec![])
        ]
    );
}

#[test]
fn doc_properties_from_core_and_app() {
    let props = parse_bytes(&deck()).expect("parse").presentation.properties;
    assert_eq!(props.title.as_deref(), Some("Deck Title"));
    assert_eq!(props.creator.as_deref(), Some("Ada"));
    assert_eq!(props.created.as_deref(), Some("2026-01-02T03:04:05Z"));
    assert_eq!(props.modified, None);
    assert_eq!(props.application.as_deref(), Some("PowerPoint"));
}

#[test]
fn hyperlinks_resolve_external_and_internal_targets() {
    let p = parse_bytes(&deck()).expect("parse").presentation;
    let Shape::TextBox(tf) = &p.slides[0].shapes[0] else {
        panic!("expected text box");
    };
    let shape_link = tf.hyperlink.as_ref().expect("shape-level link");
    assert_eq!(shape_link.url.as_deref(), Some("mailto:x@example.com"));
    assert_eq!(shape_link.tooltip.as_deref(), Some("tip"));

    let runs = &tf.paragraphs[0].runs;
    let ext = runs[0].hyperlink.as_ref().expect("external link");
    assert_eq!(ext.url.as_deref(), Some("https://example.com/a?b=1"));
    assert_eq!(ext.slide_index, None);

    let jump = runs[1].hyperlink.as_ref().expect("slide jump");
    assert_eq!(jump.url, None);
    assert_eq!(jump.slide_index, Some(2));
    assert_eq!(jump.action.as_deref(), Some("ppaction://hlinksldjump"));

    assert!(runs[2].hyperlink.is_none());
    let next = runs[3].hyperlink.as_ref().expect("show jump");
    assert_eq!(next.slide_index, Some(1));
    assert_eq!(next.rel_id, None); // 空 r:id 视为缺失
    assert_eq!(runs[3].font.as_deref(), Some("Arial")); // 样式解析不受影响
}

#[test]
fn picture_alt_text_title_and_name() {
    let p = parse_bytes(&deck()).expect("parse").presentation;
    let Shape::Picture(pic) = &p.slides[0].shapes[1] else {
        panic!("expected picture");
    };
    assert_eq!(pic.alt_text.as_deref(), Some("A red square"));
    assert_eq!(pic.title.as_deref(), Some("Logo"));
    assert_eq!(pic.name.as_deref(), Some("Picture 2"));
}
