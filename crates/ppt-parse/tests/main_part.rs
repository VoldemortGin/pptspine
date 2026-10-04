//! 演示文稿主部件定位验收:经包根 `_rels/.rels` 的 `officeDocument` 关系找主部件(缺失 / 畸形
//! 回退 `ppt/presentation.xml`);主部件的 rels 与所有相对它解析的 Target 都基于其真实位置;
//! Target 规范化后越出包根(`..` 过多)一律拒绝,不 panic、不钻进别的部件。pptx 现场合成。

use std::io::{Cursor, Write};

use ppt_parse::{parse_bytes, resolve};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

fn rels(entries: &[(&str, &str, &str)]) -> String {
    let body: String = entries
        .iter()
        .map(|(id, ty, t)| format!(r#"<Relationship Id="{id}" Type="{REL}/{ty}" Target="{t}"/>"#))
        .collect();
    format!(r#"<Relationships xmlns="{PKG_REL}">{body}</Relationships>"#)
}

fn root_rels(target: &str) -> String {
    rels(&[("rId1", "officeDocument", target)])
}

fn presentation() -> String {
    format!(
        r#"<p:presentation {NS}><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
    )
}

fn slide(text: &str) -> String {
    format!(
        r#"<p:sld {NS}><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
<p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm></p:spPr>
<p:txBody><a:bodyPr/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
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

fn first_text(bytes: &[u8]) -> String {
    let parsed = parse_bytes(bytes).expect("parse");
    let slide = &parsed.presentation.slides[0];
    format!("{:?}", slide.shapes)
}

#[test]
fn main_part_at_a_non_default_name_is_found_via_root_rels() {
    let bytes = zip(&[
        ("_rels/.rels", root_rels("ppt/presentation2.xml")),
        ("ppt/presentation2.xml", presentation()),
        (
            "ppt/_rels/presentation2.xml.rels",
            rels(&[("rId1", "slide", "slides/slide1.xml")]),
        ),
        ("ppt/slides/slide1.xml", slide("ALT-NAME")),
    ]);
    assert!(first_text(&bytes).contains("ALT-NAME"));
}

#[test]
fn leading_slash_and_dot_segments_in_the_office_document_target_are_normalized() {
    for target in ["/ppt/presentation2.xml", "./ppt/presentation2.xml"] {
        let bytes = zip(&[
            ("_rels/.rels", root_rels(target)),
            ("ppt/presentation2.xml", presentation()),
            (
                "ppt/_rels/presentation2.xml.rels",
                rels(&[("rId1", "slide", "slides/slide1.xml")]),
            ),
            ("ppt/slides/slide1.xml", slide("NORMALIZED")),
        ]);
        assert!(first_text(&bytes).contains("NORMALIZED"), "{target}");
    }
}

/// 主部件在 `custom/`:其 rels、幻灯片、版式 / 母版 / 主题 / media 都相对它解析。
#[test]
fn main_part_outside_ppt_resolves_everything_relative_to_it() {
    let layout = format!(r#"<p:sldLayout {NS}><p:cSld><p:spTree/></p:cSld></p:sldLayout>"#);
    let master = format!(
        r#"<p:sldMaster {NS}><p:cSld><p:spTree/></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/></p:sldMaster>"#
    );
    let slide_xml = format!(
        r#"<p:sld {NS}><p:cSld><p:spTree><p:pic><p:nvPicPr><p:cNvPr id="3" name="P"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
<p:blipFill><a:blip r:embed="rId9"/></p:blipFill><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="10" cy="10"/></a:xfrm></p:spPr></p:pic>
<p:sp><p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm></p:spPr>
<p:txBody><a:bodyPr/><a:p><a:r><a:t>CUSTOM-DIR</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
    );
    let bytes = zip(&[
        ("_rels/.rels", root_rels("custom/main.xml")),
        ("custom/main.xml", presentation()),
        (
            "custom/_rels/main.xml.rels",
            rels(&[("rId1", "slide", "slides/slide1.xml")]),
        ),
        ("custom/slides/slide1.xml", slide_xml),
        (
            "custom/slides/_rels/slide1.xml.rels",
            rels(&[
                ("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"),
                ("rId9", "image", "../media/image1.png"),
            ]),
        ),
        ("custom/slideLayouts/slideLayout1.xml", layout),
        (
            "custom/slideLayouts/_rels/slideLayout1.xml.rels",
            rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]),
        ),
        ("custom/slideMasters/slideMaster1.xml", master),
        ("custom/media/image1.png", "PNGDATA".to_string()),
    ]);
    let parsed = parse_bytes(&bytes).expect("parse");
    let s = &parsed.presentation.slides[0];
    assert!(format!("{:?}", s.shapes).contains("CUSTOM-DIR"));
    assert_eq!(s.layout_name.as_deref(), Some("slideLayout1.xml"));
    assert_eq!(s.master_name.as_deref(), Some("slideMaster1.xml"));
    assert!(parsed.inherit.layouts.contains_key("slideLayout1.xml"));
    assert!(parsed.inherit.masters.contains_key("slideMaster1.xml"));
    assert!(
        parsed.media.contains_key("image1.png"),
        "{:?}",
        parsed.media.keys()
    );
    let _ = resolve(&parsed);
}

#[test]
fn missing_or_malformed_root_rels_fall_back_to_the_default_main_part() {
    let default_parts = |extra: Option<(&'static str, String)>| {
        let mut v: Vec<(&str, String)> = vec![
            ("ppt/presentation.xml", presentation()),
            (
                "ppt/_rels/presentation.xml.rels",
                rels(&[("rId1", "slide", "slides/slide1.xml")]),
            ),
            ("ppt/slides/slide1.xml", slide("FALLBACK")),
        ];
        v.extend(extra);
        zip(&v)
    };
    // `.rels` 缺失。
    assert!(first_text(&default_parts(None)).contains("FALLBACK"));
    // `.rels` 畸形。
    let broken = default_parts(Some((
        "_rels/.rels",
        "<Relationships><Relationship Id=".into(),
    )));
    assert!(first_text(&broken).contains("FALLBACK"));
    // 无 officeDocument 关系。
    let other = rels(&[("rId1", "extended-properties", "docProps/app.xml")]);
    assert!(first_text(&default_parts(Some(("_rels/.rels", other)))).contains("FALLBACK"));
    // officeDocument 指向不存在的部件。
    let dangling = default_parts(Some(("_rels/.rels", root_rels("ppt/nope.xml"))));
    assert!(first_text(&dangling).contains("FALLBACK"));
}

/// 缺省位置没有主部件、也没有 `.rels` → 仍是"缺失主部件"的类型化错误,不 panic。
#[test]
fn no_main_part_anywhere_is_a_typed_error() {
    let bytes = zip(&[("ppt/slides/slide1.xml", slide("X"))]);
    assert!(parse_bytes(&bytes).is_err());
}

/// Target 里 `..` 多于目录深度:越出包根的引用被拒绝,绝不被钳到根上去命中别的部件。
#[test]
fn targets_escaping_the_package_root_are_rejected_not_clamped() {
    let parts = |main_target: &str, slide_target: &str| {
        zip(&[
            ("_rels/.rels", root_rels(main_target)),
            ("ppt/presentation.xml", presentation()),
            (
                "ppt/_rels/presentation.xml.rels",
                rels(&[("rId1", "slide", slide_target)]),
            ),
            ("ppt/slides/slide1.xml", slide("REAL")),
            // 诱饵:若 `..` 被钳在根上,会命中这些。
            ("slides/slide1.xml", slide("DECOY")),
            ("evil.xml", presentation()),
        ])
    };
    let bytes = parts("../../evil.xml", "../../slides/slide1.xml");
    let text = first_text(&bytes);
    assert!(text.contains("REAL") && !text.contains("DECOY"), "{text}");
    // 极端深度也不 panic。
    let deep = "../".repeat(200) + "slides/slide1.xml";
    let bytes = parts(&deep, &deep);
    let text = first_text(&bytes);
    assert!(text.contains("REAL") && !text.contains("DECOY"), "{text}");
}
