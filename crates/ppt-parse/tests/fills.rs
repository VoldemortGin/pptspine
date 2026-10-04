//! 形状级填充验收:`a:blipFill`(图片填充)进入模型并带 `srcRect` / `fillRect` / `tile`,
//! `a:pattFill` 降级为两色平均色的纯色(不再是无填充),`a:grpFill` 沿父组合向上继承。
//! pptx 现场合成。

use std::io::{Cursor, Write};

use ppt_core::model::{Fill, RelRect, Shape};
use ppt_core::resolved::{ResolvedFill, ResolvedShape};
use ppt_parse::{parse_bytes, resolve};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;

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
            rels(r#"<Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>"#).into_bytes(),
        ),
        ("ppt/media/image1.png", b"not a real png".to_vec()),
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

const XFRM: &str = r#"<a:xfrm><a:off x="100" y="200"/><a:ext cx="3000" cy="2000"/></a:xfrm>"#;

/// 一个带 `fill_xml` 的自选图形(`geom` 为 prstGeom 名)。
fn sp(geom: &str, fill_xml: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="S"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
<p:spPr>{XFRM}<a:prstGeom prst="{geom}"><a:avLst/></a:prstGeom>{fill_xml}</p:spPr></p:sp>"#
    )
}

fn grp(fill_xml: &str, children: &str) -> String {
    format!(
        r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="9" name="G"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="9000" cy="9000"/><a:chOff x="0" y="0"/><a:chExt cx="9000" cy="9000"/></a:xfrm>{fill_xml}</p:grpSpPr>{children}</p:grpSp>"#
    )
}

fn model_fill(bytes: &[u8]) -> Fill {
    let parsed = parse_bytes(bytes).expect("parse");
    match &parsed.presentation.slides[0].shapes[0] {
        Shape::Auto(a) => a.fill.clone().expect("fill set"),
        other => panic!("expected auto shape, got {other:?}"),
    }
}

/// 逐层下钻组合,取最内层第一个自选图形的终态填充 / 图片填充。
fn resolved_leaf(bytes: &[u8]) -> ppt_core::resolved::ResolvedAutoShape {
    let parsed = parse_bytes(bytes).expect("parse");
    let mut shapes = resolve(&parsed).slides.remove(0).shapes;
    loop {
        match shapes.remove(0) {
            ResolvedShape::Group(g) => shapes = g.children,
            ResolvedShape::Auto(a) => return a,
            other => panic!("unexpected {other:?}"),
        }
    }
}

const BLIP: &str = r#"<a:blipFill><a:blip r:embed="rId7"/><a:srcRect l="10000" t="20000" r="30000" b="5000"/><a:stretch><a:fillRect l="1000" r="2000"/></a:stretch></a:blipFill>"#;

#[test]
fn blip_fill_enters_the_model_with_media_and_rects() {
    let Fill::Blip(b) = model_fill(&deck(&sp("rect", BLIP))) else {
        panic!("expected Fill::Blip");
    };
    assert_eq!(b.rel_id, "rId7");
    assert_eq!(b.media_name.as_deref(), Some("image1.png"));
    assert_eq!(
        b.src_rect,
        Some(RelRect {
            l: 10_000,
            t: 20_000,
            r: 30_000,
            b: 5_000
        })
    );
    assert_eq!(
        b.fill_rect,
        Some(RelRect {
            l: 1_000,
            t: 0,
            r: 2_000,
            b: 0
        })
    );
    assert!(!b.tile);
}

#[test]
fn blip_fill_tile_is_flagged() {
    let tile = r#"<a:blipFill><a:blip r:embed="rId7"/><a:tile tx="0" ty="0" sx="100000" sy="100000" flip="none" algn="tl"/></a:blipFill>"#;
    let Fill::Blip(b) = model_fill(&deck(&sp("rect", tile))) else {
        panic!("expected Fill::Blip");
    };
    assert!(b.tile);
}

#[test]
fn blip_fill_reaches_the_resolved_shape_not_a_flat_color() {
    let a = resolved_leaf(&deck(&sp("ellipse", BLIP)));
    assert_eq!(a.fill, None);
    let b = a.blip_fill.expect("blip fill resolved");
    assert_eq!(b.media_name.as_deref(), Some("image1.png"));
}

#[test]
fn patt_fill_is_no_longer_unfilled_but_the_mean_of_its_two_colors() {
    let patt = r#"<a:pattFill prst="ltUpDiag"><a:fgClr><a:srgbClr val="FF0000"/></a:fgClr><a:bgClr><a:srgbClr val="0000FF"/></a:bgClr></a:pattFill>"#;
    let bytes = deck(&sp("rect", patt));
    assert!(matches!(model_fill(&bytes), Fill::Pattern { .. }));
    let a = resolved_leaf(&bytes);
    let Some(ResolvedFill::Pattern(c)) = a.fill else {
        panic!("expected pattern fill, got {:?}", a.fill);
    };
    assert_eq!(c.rgb, [128, 0, 128]);
}

const SOLID_GREEN: &str = r#"<a:solidFill><a:srgbClr val="00FF00"/></a:solidFill>"#;
const GRP_FILL: &str = "<a:grpFill/>";

#[test]
fn grp_fill_inherits_the_enclosing_group_fill() {
    let a = resolved_leaf(&deck(&grp(SOLID_GREEN, &sp("rect", GRP_FILL))));
    let Some(ResolvedFill::Solid(c)) = a.fill else {
        panic!("{:?}", a.fill);
    };
    assert_eq!(c.rgb, [0, 255, 0]);
}

#[test]
fn grp_fill_walks_up_through_nested_groups_until_a_real_fill() {
    // 外层绿;中层自己也是 grpFill(继续上找);内层自选图形 grpFill → 绿。
    let inner = grp(GRP_FILL, &grp(GRP_FILL, &sp("rect", GRP_FILL)));
    let a = resolved_leaf(&deck(&grp(SOLID_GREEN, &inner)));
    let Some(ResolvedFill::Solid(c)) = a.fill else {
        panic!("{:?}", a.fill);
    };
    assert_eq!(c.rgb, [0, 255, 0]);
}

#[test]
fn grp_fill_nearest_real_group_fill_wins() {
    let red = r#"<a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>"#;
    let a = resolved_leaf(&deck(&grp(SOLID_GREEN, &grp(red, &sp("rect", GRP_FILL)))));
    assert_eq!(a.fill.map(|f| f.color().rgb), Some([255, 0, 0]));
}

#[test]
fn grp_fill_without_any_group_fill_is_unfilled() {
    let a = resolved_leaf(&deck(&grp("", &sp("rect", GRP_FILL))));
    assert_eq!(a.fill, None);
    // 不在任何组合里的 grpFill 同样无填充。
    assert_eq!(resolved_leaf(&deck(&sp("rect", GRP_FILL))).fill, None);
}

#[test]
fn explicit_shape_fill_beats_group_fill() {
    let red = r#"<a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>"#;
    let a = resolved_leaf(&deck(&grp(SOLID_GREEN, &sp("rect", red))));
    assert_eq!(a.fill.map(|f| f.color().rgb), Some([255, 0, 0]));
}
