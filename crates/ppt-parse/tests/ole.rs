//! OLE 对象(`a:graphicData[@uri=".../ole"] > p:oleObj`)的预览图验收:`p:oleObj > p:pic`
//! 作为图片进入模型(位置用 frame 的 xfrm),`mc:AlternateContent`(包在 graphicData 内,或
//! 整个包住 graphicFrame)里 Choice 无预览图时落到带预览图的 Fallback;无预览图保持占位框。

use std::io::{Cursor, Write};

use ppt_core::export::{presentation_markdown_with, ExportOptions};
use ppt_core::geom::Rect;
use ppt_core::model::Shape;
use ppt_parse::{parse_bytes, resolve};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"
       xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006""#;
const OLE_URI: &str = "http://schemas.openxmlformats.org/presentationml/2006/ole";
const FRAME: Rect = Rect::new(1_000_000, 1_500_000, 3_000_000, 2_000_000);

const PIC: &str = r#"<p:pic><p:nvPicPr><p:cNvPr id="3" name="Preview" descr="Worksheet preview"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
<p:blipFill><a:blip r:embed="rId2"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>
<p:spPr><a:xfrm><a:off x="5" y="6"/><a:ext cx="7" cy="8"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#;

const OLE_EMBED_ONLY: &str = r#"<p:oleObj spid="_x0000_s1026" name="Worksheet" r:id="rId3" progId="Excel.Sheet.12"><p:embed/></p:oleObj>"#;

fn ole_with_pic() -> String {
    format!(
        r#"<p:oleObj name="Worksheet" r:id="rId3" progId="Excel.Sheet.12"><p:embed/>{PIC}</p:oleObj>"#
    )
}

/// 一个 OLE graphicFrame,`inner` 是 graphicData 的内容。
fn frame(inner: &str) -> String {
    format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Object"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
<p:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></p:xfrm>
<a:graphic><a:graphicData uri="{OLE_URI}">{inner}</a:graphicData></a:graphic></p:graphicFrame>"#,
        FRAME.x, FRAME.y, FRAME.w, FRAME.h
    )
}

fn alt(choice: &str, fallback: &str) -> String {
    format!(
        r#"<mc:AlternateContent><mc:Choice Requires="v">{choice}</mc:Choice><mc:Fallback>{fallback}</mc:Fallback></mc:AlternateContent>"#
    )
}

fn deck(sp_tree: &str) -> Vec<u8> {
    let rels = |body: &str| {
        format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
        )
    };
    let parts: Vec<(&str, Vec<u8>)> = vec![
        (
            "ppt/presentation.xml",
            format!(r#"<p:presentation {NS}><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#).into_bytes(),
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            rels(r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>"#).into_bytes(),
        ),
        (
            "ppt/slides/slide1.xml",
            format!(r#"<p:sld {NS}><p:cSld><p:spTree>{sp_tree}</p:spTree></p:cSld></p:sld>"#).into_bytes(),
        ),
        (
            "ppt/slides/_rels/slide1.xml.rels",
            rels(r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.emf"/>
                <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/oleObject" Target="../embeddings/Microsoft_Excel_Sheet1.xlsx"/>"#).into_bytes(),
        ),
        ("ppt/media/image1.emf", b"\x01\x00\x00\x00 not a real emf".to_vec()),
    ];
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        for (name, body) in &parts {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(body).unwrap();
        }
        zip.finish().unwrap();
    }
    buf.into_inner()
}

fn only_shape(bytes: &[u8]) -> Shape {
    let parsed = parse_bytes(bytes).expect("parse");
    let shapes = parsed.presentation.slides[0].shapes.clone();
    assert_eq!(shapes.len(), 1, "{shapes:?}");
    shapes.into_iter().next().unwrap()
}

fn assert_preview_picture(shape: &Shape) {
    let Shape::Picture(p) = shape else {
        panic!("expected picture, got {shape:?}");
    };
    // 位置取 frame 的 xfrm(不是 p:pic 自己 spPr 里的 5,6,7,8)。
    assert_eq!(p.rect, Some(FRAME));
    assert_eq!(p.media_name.as_deref(), Some("image1.emf"));
    assert!(p.image_bytes_len > 0);
    assert_eq!(p.alt_text.as_deref(), Some("Worksheet preview"));
}

#[test]
fn direct_ole_obj_preview_becomes_picture_at_frame_rect() {
    let bytes = deck(&frame(&ole_with_pic()));
    assert_preview_picture(&only_shape(&bytes));
    let parsed = parse_bytes(&bytes).unwrap();
    let md = presentation_markdown_with(
        &parsed.presentation,
        Some(&resolve(&parsed)),
        &ExportOptions::default(),
    );
    assert!(md.contains("![Worksheet preview](image1.emf)"), "{md}");
}

#[test]
fn alternate_content_inside_graphic_data_falls_to_preview_fallback() {
    let bytes = deck(&frame(&alt(OLE_EMBED_ONLY, &ole_with_pic())));
    assert_preview_picture(&only_shape(&bytes));
}

#[test]
fn alternate_content_around_whole_frame_falls_to_preview_fallback() {
    // PowerPoint 2010+ 的常见写法:Choice 里整个 graphicFrame 只有 p:embed,Fallback 里才有 p:pic。
    let bytes = deck(&alt(&frame(OLE_EMBED_ONLY), &frame(&ole_with_pic())));
    assert_preview_picture(&only_shape(&bytes));
}

#[test]
fn ole_without_preview_keeps_placeholder_box() {
    for sp_tree in [
        frame(OLE_EMBED_ONLY),
        alt(&frame(OLE_EMBED_ONLY), &frame(OLE_EMBED_ONLY)),
        frame(&alt(OLE_EMBED_ONLY, OLE_EMBED_ONLY)),
    ] {
        let Shape::Placeholder(p) = only_shape(&deck(&sp_tree)) else {
            panic!("expected placeholder");
        };
        assert_eq!(p.rect, Some(FRAME));
        assert_eq!(p.kind.as_deref(), Some(OLE_URI));
    }
}

#[test]
fn non_ole_alternate_content_choice_still_wins() {
    // 回归:上一个 commit 的语义不变——Choice 里有真内容(普通文本框)就不取 Fallback。
    let sp = |t: &str| {
        format!(
            r#"<p:sp><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:txBody></p:sp>"#
        )
    };
    let Shape::TextBox(tf) = only_shape(&deck(&alt(&sp("choice"), &sp("fallback")))) else {
        panic!("expected textbox");
    };
    assert_eq!(tf.paragraphs[0].runs[0].text, "choice");
}
