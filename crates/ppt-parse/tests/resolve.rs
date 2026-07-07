//! B-8(theme 子系统)+ B-9(占位符继承链 / ResolvedPresentation IR)验收测试
//! (PRD-PDF-EXPORT §4、§8 B-8/B-9 绿条)。
//!
//! 全部 fixture 在测试里用 `zip` 现合成(含 slideLayout / slideMaster / theme1.xml
//! 完整继承链),不落二进制。金标 RGB 值由独立实现的手算脚本得出
//! (scratchpad `golden_colors.py`,同一数学、独立代码),其中 Office 常见组合与
//! PowerPoint 取色器真实产出一致(`8FAADC`/`2F5597`/`FBE5D6`);门限 ±2/255。

use std::io::{Cursor, Write};

use ppt_core::resolved::{ResolvedBullet, ResolvedShape, ResolvedSlide};
use ppt_parse::{parse_bytes, resolve};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

// ---- fixture 合成 -----------------------------------------------------------

const XMLNS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;

/// Office 风格主题:12 色(dk1/lt1 走 sysClr+lastClr)+ major/minor 字体(带 ea)
/// + fmtScheme(fillStyleLst:纯色 phClr / phClr+tint40 / 渐变;lnStyleLst 三档)。
const THEME1: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office">
  <a:themeElements>
    <a:clrScheme name="Office">
      <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
      <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
      <a:dk2><a:srgbClr val="44546A"/></a:dk2>
      <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
      <a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
      <a:accent3><a:srgbClr val="A5A5A5"/></a:accent3>
      <a:accent4><a:srgbClr val="FFC000"/></a:accent4>
      <a:accent5><a:srgbClr val="5B9BD5"/></a:accent5>
      <a:accent6><a:srgbClr val="70AD47"/></a:accent6>
      <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
      <a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
    </a:clrScheme>
    <a:fontScheme name="Office">
      <a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface="DengXian Light"/><a:cs typeface=""/></a:majorFont>
      <a:minorFont><a:latin typeface="Calibri"/><a:ea typeface="DengXian"/><a:cs typeface=""/></a:minorFont>
    </a:fontScheme>
    <a:fmtScheme name="Office">
      <a:fillStyleLst>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
        <a:solidFill><a:schemeClr val="phClr"><a:tint val="40000"/></a:schemeClr></a:solidFill>
        <a:gradFill><a:gsLst><a:gs pos="0"><a:schemeClr val="phClr"/></a:gs></a:gsLst></a:gradFill>
      </a:fillStyleLst>
      <a:lnStyleLst>
        <a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>
        <a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"><a:shade val="50000"/></a:schemeClr></a:solidFill></a:ln>
        <a:ln w="19050"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>
      </a:lnStyleLst>
      <a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>
      <a:bgFillStyleLst>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
      </a:bgFillStyleLst>
    </a:fmtScheme>
  </a:themeElements>
</a:theme>"#;

/// master:title / body 两个占位符(都带 xfrm),clrMap,txStyles 三桶;
/// title 占位符自带 lstStyle(`b="1"`,继承链的 master-ph 层)。
fn master1() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster {XMLNS}>
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Title Placeholder 1"/><p:cNvSpPr/>
          <p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="838200" y="365125"/><a:ext cx="7772400" cy="1325563"/></a:xfrm></p:spPr>
        <p:txBody>
          <a:bodyPr/>
          <a:lstStyle><a:lvl1pPr><a:defRPr b="1"/></a:lvl1pPr></a:lstStyle>
          <a:p><a:r><a:t>Click to edit Master title style</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Body Placeholder 2"/><p:cNvSpPr/>
          <p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="838200" y="1825625"/><a:ext cx="7772400" cy="4351338"/></a:xfrm></p:spPr>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>master body prompt</a:t></a:r></a:p></p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
  <p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2"
            accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6"
            hlink="hlink" folHlink="folHlink"/>
  <p:txStyles>
    <p:titleStyle>
      <a:lvl1pPr algn="ctr"><a:buNone/>
        <a:defRPr sz="4400"><a:solidFill><a:schemeClr val="tx2"/></a:solidFill><a:latin typeface="+mj-lt"/></a:defRPr>
      </a:lvl1pPr>
    </p:titleStyle>
    <p:bodyStyle>
      <a:lvl1pPr marL="342900" indent="-342900"><a:buFont typeface="Arial"/><a:buChar char="&#8226;"/>
        <a:defRPr sz="2800"/></a:lvl1pPr>
      <a:lvl2pPr marL="742950" indent="-285750"><a:buFont typeface="Arial"/><a:buSzPct val="75000"/><a:buChar char="&#8211;"/>
        <a:defRPr sz="2400"/></a:lvl2pPr>
    </p:bodyStyle>
    <p:otherStyle>
      <a:lvl1pPr><a:defRPr sz="1800"/></a:lvl1pPr>
    </p:otherStyle>
  </p:txStyles>
</p:sldMaster>"#
    )
}

/// layout:title 占位符带自己的 xfrm + lstStyle(sz=4000 i=1 algn=l,更近层);
/// body 占位符**无 xfrm**(几何应落到 master)。
fn layout1() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout {XMLNS}>
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/>
          <p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="1000000" y="500000"/><a:ext cx="7000000" cy="1200000"/></a:xfrm></p:spPr>
        <p:txBody><a:bodyPr/>
          <a:lstStyle><a:lvl1pPr algn="l"><a:defRPr sz="4000" i="1"/></a:lvl1pPr></a:lstStyle>
          <a:p><a:r><a:t>layout title prompt</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Content 2"/><p:cNvSpPr/>
          <p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p/></p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
  <p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
</p:sldLayout>"#
    )
}

/// 缺省测试 slide:ctrTitle(无 xfrm、无 rPr,等价类匹配 title)
/// + idx=1 无 type 占位符(缺省 body),第二段 lvl=1。
fn slide_default() -> String {
    slide_with(
        r#"<p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/>
          <p:nvPr><p:ph type="ctrTitle"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/><a:p><a:r><a:t>Deck Title</a:t></a:r></a:p></p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Content 2"/><p:cNvSpPr/>
          <p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/>
          <a:p><a:r><a:t>first level</a:t></a:r></a:p>
          <a:p><a:pPr lvl="1"/><a:r><a:t>second level</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>"#,
        "",
    )
}

/// 用给定 spTree 内容(+ 可选 `p:sld` 级尾巴,如 clrMapOvr)合成 slide XML。
fn slide_with(sp_tree_inner: &str, after_csld: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld {XMLNS}>
  <p:cSld><p:spTree>{sp_tree_inner}</p:spTree></p:cSld>{after_csld}
</p:sld>"#
    )
}

/// 把完整继承链部件打成内存 `.pptx`(presentation → slide → layout → master → theme)。
fn build_deck(slide_xml: &str, presentation_extra: &str) -> Vec<u8> {
    build_deck_parts(slide_xml, presentation_extra, &layout1(), &master1())
}

/// 同 [`build_deck`],但 layout / master XML 可自定义(背景继承测试用)。
fn build_deck_parts(
    slide_xml: &str,
    presentation_extra: &str,
    layout_xml: &str,
    master_xml: &str,
) -> Vec<u8> {
    let presentation = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation {XMLNS}>
  <p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId2"/></p:sldMasterIdLst>
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
  <p:sldSz cx="9144000" cy="6858000" type="screen4x3"/>{presentation_extra}
</p:presentation>"#
    );
    let parts: Vec<(&str, String)> = vec![
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
</Types>"#
                .into(),
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"#
                .into(),
        ),
        ("ppt/presentation.xml", presentation),
        (
            "ppt/_rels/presentation.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>
</Relationships>"#
                .into(),
        ),
        ("ppt/slides/slide1.xml", slide_xml.into()),
        (
            "ppt/slides/_rels/slide1.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
</Relationships>"#
                .into(),
        ),
        ("ppt/slideLayouts/slideLayout1.xml", layout_xml.into()),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/>
</Relationships>"#
                .into(),
        ),
        ("ppt/slideMasters/slideMaster1.xml", master_xml.into()),
        (
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/>
</Relationships>"#
                .into(),
        ),
        ("ppt/theme/theme1.xml", THEME1.into()),
    ];
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default();
        for (name, body) in &parts {
            zip.start_file(*name, opts).expect("start_file");
            zip.write_all(body.as_bytes()).expect("write");
        }
        zip.finish().expect("finish zip");
    }
    buf.into_inner()
}

/// 解析 + 继承链解析,返回唯一一张 ResolvedSlide。
fn resolve_slide(slide_xml: &str, presentation_extra: &str) -> ResolvedSlide {
    let parsed = parse_bytes(&build_deck(slide_xml, presentation_extra)).expect("parse deck");
    let resolved = resolve(&parsed);
    resolved.slides.into_iter().next().expect("one slide")
}

/// 同 [`resolve_slide`],但 layout / master XML 可自定义(背景继承测试用)。
fn resolve_slide_parts(slide_xml: &str, layout_xml: &str, master_xml: &str) -> ResolvedSlide {
    let parsed =
        parse_bytes(&build_deck_parts(slide_xml, "", layout_xml, master_xml)).expect("parse deck");
    let resolved = resolve(&parsed);
    resolved.slides.into_iter().next().expect("one slide")
}

/// 一个只带 `p:bg` 的最小 slideLayout(背景继承测试用,spTree 留空)。
fn layout_with_bg(bg_inner: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout {XMLNS}>
  <p:cSld>
    <p:bg>{bg_inner}</p:bg>
    <p:spTree/>
  </p:cSld>
</p:sldLayout>"#
    )
}

/// 一个只带 `p:bg` 的最小 slideMaster(背景继承测试用,spTree 留空)。
fn master_with_bg(bg_inner: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster {XMLNS}>
  <p:cSld>
    <p:bg>{bg_inner}</p:bg>
    <p:spTree/>
  </p:cSld>
</p:sldMaster>"#
    )
}

/// 一个带 `p:bg` 的 slide(sp_tree_inner 通常留空;背景继承优先级测试用)。
fn slide_with_bg(bg_inner: &str, sp_tree_inner: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld {XMLNS}>
  <p:cSld>
    <p:bg>{bg_inner}</p:bg>
    <p:spTree>{sp_tree_inner}</p:spTree>
  </p:cSld>
</p:sld>"#
    )
}

/// `p:bgPr > a:solidFill`(纯色背景)片段。
fn solid_bg(hex: &str) -> String {
    format!(r#"<p:bgPr><a:solidFill><a:srgbClr val="{hex}"/></a:solidFill></p:bgPr>"#)
}

fn resolve_default() -> ResolvedSlide {
    resolve_slide(&slide_default(), "")
}

/// ±2/255 金标断言(PRD B-8 门限)。
fn assert_rgb_within(got: [u8; 3], want: [u8; 3], what: &str) {
    for i in 0..3 {
        let d = (got[i] as i32 - want[i] as i32).abs();
        assert!(d <= 2, "{what}: channel {i} got {got:?}, want {want:?}");
    }
}

fn as_text_box(shape: &ResolvedShape) -> &ppt_core::resolved::ResolvedTextFrame {
    match shape {
        ResolvedShape::TextBox(tf) => tf,
        other => panic!("expected resolved text box, got {other:?}"),
    }
}

// ---- 几何继承(slide → layout → master)------------------------------------

/// B-9 绿条核心:slide 标题**无 xfrm 无 rPr** → 矩形取 layout 的 title 占位符。
#[test]
fn placeholder_geometry_falls_back_to_layout() {
    let slide = resolve_default();
    let title = as_text_box(&slide.shapes[0]);
    let rect = title.rect.expect("title rect materialized from layout");
    assert_eq!(
        (rect.x, rect.y, rect.w, rect.h),
        (1_000_000, 500_000, 7_000_000, 1_200_000)
    );
}

/// layout 占位符也无 xfrm → 矩形继续落到 master。
#[test]
fn placeholder_geometry_falls_back_to_master() {
    let slide = resolve_default();
    let body = as_text_box(&slide.shapes[1]);
    let rect = body.rect.expect("body rect materialized from master");
    assert_eq!(
        (rect.x, rect.y, rect.w, rect.h),
        (838_200, 1_825_625, 7_772_400, 4_351_338)
    );
}

/// slide 自己的 xfrm 整体获胜(不逐字段合并)。
#[test]
fn slide_own_xfrm_wins() {
    let slide_xml = slide_with(
        r#"<p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="111" y="222"/><a:ext cx="333" cy="444"/></a:xfrm></p:spPr>
        <p:txBody><a:bodyPr/><a:p><a:r><a:t>t</a:t></a:r></a:p></p:txBody>
      </p:sp>"#,
        "",
    );
    let slide = resolve_slide(&slide_xml, "");
    let rect = as_text_box(&slide.shapes[0]).rect.expect("own rect");
    assert_eq!((rect.x, rect.y, rect.w, rect.h), (111, 222, 333, 444));
}

// ---- 背景继承(slide → layout → master)-------------------------------------

/// slide 无 `p:bg` → 落到 layout 的纯色背景。
#[test]
fn background_falls_back_to_layout() {
    let slide_xml = slide_with("", "");
    let slide = resolve_slide_parts(&slide_xml, &layout_with_bg(&solid_bg("00FF00")), &master1());
    let bg = slide.background.expect("background from layout");
    let ppt_core::resolved::ResolvedBackground::Color(fill) = bg else {
        panic!("expected color background");
    };
    assert_eq!(fill.color().rgb, [0x00, 0xFF, 0x00], "layout bg 回退");
}

/// slide 与 layout 皆无 `p:bg` → 落到 master 的纯色背景。
#[test]
fn background_falls_back_to_master() {
    let slide_xml = slide_with("", "");
    let slide = resolve_slide_parts(&slide_xml, &layout1(), &master_with_bg(&solid_bg("FF00FF")));
    let bg = slide.background.expect("background from master");
    let ppt_core::resolved::ResolvedBackground::Color(fill) = bg else {
        panic!("expected color background");
    };
    assert_eq!(fill.color().rgb, [0xFF, 0x00, 0xFF], "master bg 回退");
}

/// slide 自己的 `p:bg` 整体获胜,即便 layout / master 都另有背景。
#[test]
fn background_slide_own_wins_over_layout_and_master() {
    let slide_xml = slide_with_bg(&solid_bg("0000FF"), "");
    let slide = resolve_slide_parts(
        &slide_xml,
        &layout_with_bg(&solid_bg("00FF00")),
        &master_with_bg(&solid_bg("FF0000")),
    );
    let bg = slide.background.expect("slide own background");
    let ppt_core::resolved::ResolvedBackground::Color(fill) = bg else {
        panic!("expected color background");
    };
    assert_eq!(
        fill.color().rgb,
        [0x00, 0x00, 0xFF],
        "slide bg 优先于 layout/master"
    );
}

// ---- 文本样式继承链(逐级合并 + 等价类匹配)---------------------------------

/// B-9 绿条核心:标题 run 无任何直接格式化,样式全部来自链——
/// 逐属性合并:layout ph lstStyle(sz=4000 i=1 algn=l)覆盖 master txStyles
/// titleStyle(sz=4400 algn=ctr),master ph lstStyle 提供 b=1(层间保留);
/// 颜色 schemeClr tx2 经 clrMap 映到 dk2;字体 `+mj-lt` 展开为主题 major latin。
/// slide 的 `ctrTitle` 与 layout/master 的 `title` 按等价类匹配。
#[test]
fn title_style_merges_through_chain() {
    let slide = resolve_default();
    let title = as_text_box(&slide.shapes[0]);
    let para = &title.paragraphs[0];
    let run = &para.runs[0];
    assert_eq!(run.text, "Deck Title");
    assert_eq!(
        run.size_pt, 40.0,
        "layout lstStyle sz 覆盖 master titleStyle"
    );
    assert!(run.italic, "layout lstStyle i=1");
    assert!(run.bold, "master ph lstStyle b=1 在更近层无覆盖时保留");
    assert_eq!(
        para.align.as_deref(),
        Some("l"),
        "layout algn 覆盖 master ctr"
    );
    assert_eq!(run.font.as_deref(), Some("Calibri Light"), "+mj-lt 展开");
    assert_rgb_within(run.color.rgb, [0x44, 0x54, 0x6A], "tx2 -> clrMap -> dk2");
    // titleStyle buNone → 标题无项目符号。
    assert_eq!(para.bullet, ResolvedBullet::None);
}

/// 层级选层:lvl=1 段落取 bodyStyle lvl2pPr(字号 / 缩进 / 符号字符+字体+大小);
/// lvl=0 段落取 lvl1pPr。idx=1 无 type 占位符按缺省 body 匹配。
#[test]
fn body_level_selection_and_bullets() {
    let slide = resolve_default();
    let body = as_text_box(&slide.shapes[1]);

    let p0 = &body.paragraphs[0];
    assert_eq!(p0.runs[0].size_pt, 28.0, "lvl1pPr defRPr sz");
    assert_eq!(p0.mar_l, Some(342_900));
    assert_eq!(p0.indent, Some(-342_900));
    assert_eq!(
        p0.bullet,
        ResolvedBullet::Char {
            ch: "\u{2022}".into(),
            font: Some("Arial".into()),
            size_pct: None,
        },
        "master bodyStyle lvl1 bullet 继承到 slide"
    );

    let p1 = &body.paragraphs[1];
    assert_eq!(p1.level, 1);
    assert_eq!(p1.runs[0].size_pt, 24.0, "lvl2pPr defRPr sz");
    assert_eq!(p1.mar_l, Some(742_950));
    assert_eq!(p1.indent, Some(-285_750));
    assert_eq!(
        p1.bullet,
        ResolvedBullet::Char {
            ch: "\u{2013}".into(),
            font: Some("Arial".into()),
            size_pct: Some(0.75),
        },
        "B-9 绿条:lvl2 body bullet 继承 master 符号字符(含 buSzPct)"
    );
}

/// run 直接格式化永远最后获胜(逐属性:显式字段覆盖,未指定字段仍继承)。
#[test]
fn run_direct_formatting_wins() {
    let slide_xml = slide_with(
        r#"<p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/>
          <a:p><a:r>
            <a:rPr sz="1200" b="0"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:rPr>
            <a:t>direct</a:t>
          </a:r></a:p>
        </p:txBody>
      </p:sp>"#,
        "",
    );
    let slide = resolve_slide(&slide_xml, "");
    let run = &as_text_box(&slide.shapes[0]).paragraphs[0].runs[0];
    assert_eq!(run.size_pt, 12.0, "run sz 覆盖整条链");
    assert!(!run.bold, "run b=0 显式覆盖 master ph lstStyle b=1");
    assert!(run.italic, "未指定属性仍从 layout 继承");
    assert_rgb_within(run.color.rgb, [0xFF, 0x00, 0x00], "run 直接颜色");
}

/// slide txBody 自带 lstStyle 覆盖 layout/master;更近层 buNone **压制**继承符号。
#[test]
fn slide_lst_style_overrides_and_bu_none_suppresses() {
    let slide_xml = slide_with(
        r#"<p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="C"/><p:cNvSpPr/><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/>
          <a:lstStyle>
            <a:lvl1pPr><a:buNone/><a:defRPr sz="2000"/></a:lvl1pPr>
          </a:lstStyle>
          <a:p><a:r><a:t>no bullet here</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>"#,
        "",
    );
    let slide = resolve_slide(&slide_xml, "");
    let para = &as_text_box(&slide.shapes[0]).paragraphs[0];
    assert_eq!(para.bullet, ResolvedBullet::None, "buNone 压制 master 符号");
    assert_eq!(
        para.runs[0].size_pt, 20.0,
        "slide lstStyle sz 覆盖 master 2800"
    );
    assert_eq!(para.mar_l, Some(342_900), "未覆盖的缩进仍继承 master");
}

/// 段落 pPr 的 buNone(比 lstStyle 更近)同样压制;段落 defRPr 参与 run 合并。
#[test]
fn paragraph_ppr_is_nearest_paragraph_source() {
    let slide_xml = slide_with(
        r#"<p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="C"/><p:cNvSpPr/><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/>
          <a:p>
            <a:pPr algn="r"><a:buNone/><a:defRPr u="sng"/></a:pPr>
            <a:r><a:t>para direct</a:t></a:r>
          </a:p>
        </p:txBody>
      </p:sp>"#,
        "",
    );
    let slide = resolve_slide(&slide_xml, "");
    let para = &as_text_box(&slide.shapes[0]).paragraphs[0];
    assert_eq!(para.bullet, ResolvedBullet::None);
    assert_eq!(para.align.as_deref(), Some("r"));
    assert!(para.runs[0].underline, "段落 defRPr u=sng 落到 run");
    assert_eq!(para.runs[0].size_pt, 28.0, "未覆盖字号仍走 master lvl1");
}

/// 非占位符文本框:master otherStyle(sz=1800)+ presentation defaultTextStyle
/// (sz=2000,更近的文档级缺省)作基链 → 2000 获胜。
#[test]
fn non_placeholder_uses_default_text_style_base() {
    let slide_xml = slide_with(
        r#"<p:sp>
        <p:spPr/>
        <p:txBody><a:bodyPr/><a:p><a:r><a:t>plain box</a:t></a:r></a:p></p:txBody>
      </p:sp>"#,
        "",
    );
    let extra = r#"
  <p:defaultTextStyle><a:lvl1pPr><a:defRPr sz="2000"/></a:lvl1pPr></p:defaultTextStyle>"#;
    let slide = resolve_slide(&slide_xml, extra);
    let run = &as_text_box(&slide.shapes[0]).paragraphs[0].runs[0];
    assert_eq!(run.size_pt, 20.0, "defaultTextStyle 覆盖 otherStyle");

    // 没有 defaultTextStyle 时落 otherStyle。
    let slide2 = resolve_slide(&slide_xml, "");
    let run2 = &as_text_box(&slide2.shapes[0]).paragraphs[0].runs[0];
    assert_eq!(run2.size_pt, 18.0, "otherStyle sz=1800");
}

// ---- 颜色:clrMap / clrMapOvr / 变换金标(B-8)------------------------------

/// schemeClr 的映射名经 clrMap 重映射:tx1→dk1(sysClr lastClr 000000)、
/// bg1→lt1(FFFFFF)、直接槽位名原样。
#[test]
fn scheme_color_remaps_through_clr_map() {
    let slide_xml = slide_with(
        r#"<p:sp><p:spPr/><p:txBody><a:bodyPr/>
          <a:p>
            <a:r><a:rPr><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:rPr><a:t>a</a:t></a:r>
            <a:r><a:rPr><a:solidFill><a:schemeClr val="bg1"/></a:solidFill></a:rPr><a:t>b</a:t></a:r>
            <a:r><a:rPr><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></a:rPr><a:t>c</a:t></a:r>
          </a:p>
        </p:txBody></p:sp>"#,
        "",
    );
    let slide = resolve_slide(&slide_xml, "");
    let runs = &as_text_box(&slide.shapes[0]).paragraphs[0].runs;
    assert_eq!(
        runs[0].color.rgb,
        [0x00, 0x00, 0x00],
        "tx1 -> dk1(sysClr lastClr)"
    );
    assert_eq!(runs[1].color.rgb, [0xFF, 0xFF, 0xFF], "bg1 -> lt1");
    assert_eq!(runs[2].color.rgb, [0x44, 0x72, 0xC4], "accent1 直取");
}

/// slide 级 `clrMapOvr > overrideClrMapping` 覆盖 master 的 clrMap:
/// tx2 改映 accent6 后,标题(schemeClr tx2)解析为 70AD47 而非 dk2。
#[test]
fn slide_clr_map_ovr_overrides_master_map() {
    let ovr = r#"
  <p:clrMapOvr><a:overrideClrMapping bg1="lt1" tx1="dk1" bg2="lt2" tx2="accent6"
    accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4"
    accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/></p:clrMapOvr>"#;
    let slide_xml = slide_with(
        r#"<p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/><a:p><a:r><a:t>remapped</a:t></a:r></a:p></p:txBody>
      </p:sp>"#,
        ovr,
    );
    let slide = resolve_slide(&slide_xml, "");
    let run = &as_text_box(&slide.shapes[0]).paragraphs[0].runs[0];
    assert_rgb_within(
        run.color.rgb,
        [0x70, 0xAD, 0x47],
        "tx2 -> accent6(覆盖映射)",
    );
}

/// 颜色变换金标表(全链:schemeClr → clrScheme → 变换;±2/255)。
/// 手算值见 scratchpad `golden_colors.py`;Lighter 40% / Darker 25% / Lighter 80%
/// 与 PowerPoint 取色器真实产出一致。
#[test]
fn color_transform_golden_table_end_to_end() {
    let slide_xml = slide_with(
        r#"<p:sp><p:spPr/><p:txBody><a:bodyPr/>
          <a:p>
            <a:r><a:rPr><a:solidFill><a:schemeClr val="accent1"><a:lumMod val="60000"/><a:lumOff val="40000"/></a:schemeClr></a:solidFill></a:rPr><a:t>1</a:t></a:r>
            <a:r><a:rPr><a:solidFill><a:schemeClr val="accent1"><a:lumMod val="75000"/></a:schemeClr></a:solidFill></a:rPr><a:t>2</a:t></a:r>
            <a:r><a:rPr><a:solidFill><a:schemeClr val="accent1"><a:tint val="40000"/></a:schemeClr></a:solidFill></a:rPr><a:t>3</a:t></a:r>
            <a:r><a:rPr><a:solidFill><a:schemeClr val="accent1"><a:shade val="50000"/></a:schemeClr></a:solidFill></a:rPr><a:t>4</a:t></a:r>
            <a:r><a:rPr><a:solidFill><a:schemeClr val="accent2"><a:lumMod val="20000"/><a:lumOff val="80000"/></a:schemeClr></a:solidFill></a:rPr><a:t>5</a:t></a:r>
            <a:r><a:rPr><a:solidFill><a:srgbClr val="FF0000"><a:satMod val="50000"/></a:srgbClr></a:solidFill></a:rPr><a:t>6</a:t></a:r>
            <a:r><a:rPr><a:solidFill><a:schemeClr val="accent1"><a:alpha val="50000"/></a:schemeClr></a:solidFill></a:rPr><a:t>7</a:t></a:r>
          </a:p>
        </p:txBody></p:sp>"#,
        "",
    );
    let slide = resolve_slide(&slide_xml, "");
    let runs = &as_text_box(&slide.shapes[0]).paragraphs[0].runs;
    let golden: [(usize, [u8; 3], &str); 6] = [
        (
            0,
            [0x8F, 0xAA, 0xDC],
            "accent1 Lighter 40% (lumMod60+lumOff40)",
        ),
        (1, [0x2F, 0x55, 0x97], "accent1 Darker 25% (lumMod75)"),
        (2, [0xCF, 0xD5, 0xEA], "accent1 tint40"),
        (3, [0x2F, 0x52, 0x8F], "accent1 shade50"),
        (
            4,
            [0xFB, 0xE5, 0xD6],
            "accent2 Lighter 80% (lumMod20+lumOff80)",
        ),
        (5, [0xBF, 0x40, 0x40], "srgb FF0000 satMod50"),
    ];
    for (i, want, what) in golden {
        assert_rgb_within(runs[i].color.rgb, want, what);
        assert_eq!(runs[i].color.alpha, None, "{what}: 无 alpha");
    }
    // alpha:RGB 不变,透明度带出。
    assert_eq!(runs[6].color.rgb, [0x44, 0x72, 0xC4]);
    assert_eq!(runs[6].color.alpha, Some(0.5));
}

// ---- 字体:主题引用展开(B-8)-----------------------------------------------

/// `+mn-lt`/`+mn-ea` 展开为主题 minor 字体(B-8 绿条);普通名原样。
#[test]
fn theme_font_refs_expand() {
    let slide_xml = slide_with(
        r#"<p:sp><p:spPr/><p:txBody><a:bodyPr/>
          <a:p>
            <a:r><a:rPr><a:latin typeface="+mn-lt"/><a:ea typeface="+mn-ea"/></a:rPr><a:t>minor</a:t></a:r>
            <a:r><a:rPr><a:latin typeface="+mj-lt"/><a:ea typeface="+mj-ea"/></a:rPr><a:t>major</a:t></a:r>
            <a:r><a:rPr><a:latin typeface="Consolas"/></a:rPr><a:t>literal</a:t></a:r>
          </a:p>
        </p:txBody></p:sp>"#,
        "",
    );
    let slide = resolve_slide(&slide_xml, "");
    let runs = &as_text_box(&slide.shapes[0]).paragraphs[0].runs;
    assert_eq!(runs[0].font.as_deref(), Some("Calibri"));
    assert_eq!(runs[0].ea_font.as_deref(), Some("DengXian"));
    assert_eq!(runs[1].font.as_deref(), Some("Calibri Light"));
    assert_eq!(runs[1].ea_font.as_deref(), Some("DengXian Light"));
    assert_eq!(runs[2].font.as_deref(), Some("Consolas"));
}

// ---- p:style 形状样式引用(B-8:fillRef / lnRef / fontRef)-------------------

/// fillRef:idx=1(纯色 phClr)→ 引用色;idx=2(phClr+tint40)→ 变换后;
/// idx=3(渐变)→ 降级为代表色(引用色本身)。显式 spPr 填充仍获胜。
#[test]
fn fill_ref_resolves_from_theme_format_lists() {
    let slide_xml = slide_with(
        r#"<p:sp>
        <p:spPr><a:prstGeom prst="rect"/></p:spPr>
        <p:style><a:lnRef idx="0"/><a:fillRef idx="1"><a:schemeClr val="accent2"/></a:fillRef></p:style>
      </p:sp>
      <p:sp>
        <p:spPr><a:prstGeom prst="rect"/></p:spPr>
        <p:style><a:fillRef idx="2"><a:schemeClr val="accent2"/></a:fillRef></p:style>
      </p:sp>
      <p:sp>
        <p:spPr><a:prstGeom prst="rect"/></p:spPr>
        <p:style><a:fillRef idx="3"><a:schemeClr val="accent2"/></a:fillRef></p:style>
      </p:sp>
      <p:sp>
        <p:spPr><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="112233"/></a:solidFill></p:spPr>
        <p:style><a:fillRef idx="1"><a:schemeClr val="accent2"/></a:fillRef></p:style>
      </p:sp>"#,
        "",
    );
    let slide = resolve_slide(&slide_xml, "");
    let fills: Vec<[u8; 3]> = slide
        .shapes
        .iter()
        .map(|s| match s {
            ResolvedShape::Auto(a) => a.fill.expect("fill resolved").color().rgb,
            other => panic!("expected auto shape, got {other:?}"),
        })
        .collect();
    assert_rgb_within(fills[0], [0xED, 0x7D, 0x31], "fillRef idx=1 纯色 phClr");
    assert_rgb_within(fills[1], [0xF8, 0xD7, 0xCD], "fillRef idx=2 phClr+tint40");
    assert_rgb_within(
        fills[2],
        [0xED, 0x7D, 0x31],
        "fillRef idx=3 渐变降级为代表色",
    );
    assert_rgb_within(fills[3], [0x11, 0x22, 0x33], "显式 spPr 填充获胜");
}

/// lnRef:主题 lnStyleLst 第 2 档(w=12700,phClr+shade50)→ 连接线描边。
#[test]
fn ln_ref_resolves_theme_line() {
    let slide_xml = slide_with(
        r#"<p:cxnSp>
        <p:nvCxnSpPr><p:cNvPr id="4" name="Conn"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm>
          <a:prstGeom prst="line"/></p:spPr>
        <p:style><a:lnRef idx="2"><a:schemeClr val="accent1"/></a:lnRef></p:style>
      </p:cxnSp>"#,
        "",
    );
    let slide = resolve_slide(&slide_xml, "");
    let ResolvedShape::Connector(c) = &slide.shapes[0] else {
        panic!("expected connector");
    };
    let stroke = c.stroke.as_ref().expect("stroke from lnRef");
    assert_eq!(stroke.width_emu, Some(12_700), "主题线宽");
    assert_rgb_within(
        stroke.color.expect("stroke color").rgb,
        [0x2F, 0x52, 0x8F],
        "phClr=accent1 + shade50(主题 ln 档内变换)",
    );
}

/// fontRef:链上无字体/颜色时落 `p:style > a:fontRef`(minor 字体 + 引用色)。
#[test]
fn font_ref_is_weakest_font_and_color_source() {
    let slide_xml = slide_with(
        r#"<p:sp>
        <p:spPr/>
        <p:style><a:fontRef idx="minor"><a:schemeClr val="accent5"/></a:fontRef></p:style>
        <p:txBody><a:bodyPr/><a:p><a:r><a:t>styled text</a:t></a:r></a:p></p:txBody>
      </p:sp>"#,
        "",
    );
    let slide = resolve_slide(&slide_xml, "");
    let run = &as_text_box(&slide.shapes[0]).paragraphs[0].runs[0];
    assert_eq!(
        run.font.as_deref(),
        Some("Calibri"),
        "fontRef minor -> latin"
    );
    assert_eq!(
        run.ea_font.as_deref(),
        Some("DengXian"),
        "fontRef minor -> ea"
    );
    assert_rgb_within(run.color.rgb, [0x5B, 0x9B, 0xD5], "fontRef 子颜色兜底");
}

// ---- 表格单元格 scheme 填充 --------------------------------------------------

/// 单元格 scheme 填充(含变换)终端化;文字走非占位符基链(otherStyle)。
#[test]
fn table_cell_scheme_fill_resolves() {
    let slide_xml = slide_with(
        r#"<p:graphicFrame>
        <p:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></p:xfrm>
        <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
          <a:tbl>
            <a:tblGrid><a:gridCol w="50"/></a:tblGrid>
            <a:tr h="10"><a:tc>
              <a:txBody><a:p><a:r><a:t>cell</a:t></a:r></a:p></a:txBody>
              <a:tcPr><a:solidFill><a:schemeClr val="accent6"><a:lumMod val="75000"/></a:schemeClr></a:solidFill></a:tcPr>
            </a:tc></a:tr>
          </a:tbl>
        </a:graphicData></a:graphic>
      </p:graphicFrame>"#,
        "",
    );
    let slide = resolve_slide(&slide_xml, "");
    let ResolvedShape::Table(t) = &slide.shapes[0] else {
        panic!("expected table");
    };
    let cell = &t.rows[0].cells[0];
    // 70AD47 lumMod75 手算 -> 548235(golden_colors.py)。
    assert_rgb_within(
        cell.fill.expect("cell fill").rgb,
        [0x54, 0x82, 0x35],
        "accent6 Darker 25%",
    );
    assert_eq!(cell.paragraphs[0].runs[0].size_pt, 18.0, "otherStyle 基链");
}

// ---- 解析层捕获(B-8/B-9 的 parse 面)---------------------------------------

/// 继承链部件全部落进 `ParsedPptx.inherit`:theme 12 色(sysClr 折 lastClr)、
/// 主题字体(含 ea)、fmtScheme 列表、master clrMap/txStyles、layout 形状。
#[test]
fn inheritance_parts_are_captured() {
    let parsed = parse_bytes(&build_deck(&slide_default(), "")).expect("parse deck");
    let inherit = &parsed.inherit;

    let theme = inherit.themes.get("theme1.xml").expect("theme parsed");
    assert_eq!(
        theme.color_scheme.dk1.rgb,
        [0x00, 0x00, 0x00],
        "sysClr lastClr"
    );
    assert_eq!(theme.color_scheme.lt1.rgb, [0xFF, 0xFF, 0xFF]);
    assert_eq!(theme.color_scheme.accent1.rgb, [0x44, 0x72, 0xC4]);
    assert_eq!(theme.color_scheme.fol_hlink.rgb, [0x95, 0x4F, 0x72]);
    assert_eq!(
        theme.font_scheme.major.latin.as_deref(),
        Some("Calibri Light")
    );
    assert_eq!(theme.font_scheme.minor.ea.as_deref(), Some("DengXian"));
    assert_eq!(theme.font_scheme.minor.cs, None, "空串 typeface 按缺省");
    assert_eq!(theme.fill_styles.len(), 3);
    assert!(theme.fill_styles[0].is_some() && theme.fill_styles[1].is_some());
    assert!(theme.fill_styles[2].is_none(), "渐变项记 None(降级)");
    assert_eq!(theme.line_styles.len(), 3);
    assert_eq!(theme.line_styles[1].width_emu, Some(12_700));

    let master = inherit
        .masters
        .get("slideMaster1.xml")
        .expect("master parsed");
    let clr_map = master.clr_map.as_ref().expect("clrMap");
    assert_eq!(clr_map.map("tx1"), "dk1");
    let tx = master.tx_styles.as_ref().expect("txStyles");
    assert_eq!(
        tx.title
            .level(0)
            .and_then(|l| l.def_rpr.as_ref())
            .and_then(|r| r.size_pt),
        Some(44.0)
    );
    assert_eq!(
        tx.body
            .level(1)
            .and_then(|l| l.def_rpr.as_ref())
            .and_then(|r| r.size_pt),
        Some(24.0)
    );
    assert_eq!(master.theme_name.as_deref(), Some("theme1.xml"));

    let layout = inherit
        .layouts
        .get("slideLayout1.xml")
        .expect("layout parsed");
    assert_eq!(layout.master_name.as_deref(), Some("slideMaster1.xml"));
    assert!(layout.clr_map_ovr.is_none(), "masterClrMapping -> 沿用上级");
    assert_eq!(layout.shapes.len(), 2, "layout spTree 形状可供匹配");
}

/// slide 形状的占位符标识(type/idx)在解析层被捕获。
#[test]
fn slide_placeholder_refs_are_captured() {
    let parsed = parse_bytes(&build_deck(&slide_default(), "")).expect("parse deck");
    let shapes = &parsed.presentation.slides[0].shapes;
    let ppt_core::model::Shape::TextBox(title) = &shapes[0] else {
        panic!("expected text box");
    };
    let ph = title.placeholder.as_ref().expect("ph captured");
    assert_eq!(ph.kind.as_deref(), Some("ctrTitle"));
    assert_eq!(ph.idx, None);
    let ppt_core::model::Shape::TextBox(body) = &shapes[1] else {
        panic!("expected text box");
    };
    let ph = body.placeholder.as_ref().expect("ph captured");
    assert_eq!(ph.kind, None);
    assert_eq!(ph.idx, Some(1));
}

/// 没有 layout/master/theme 的最小 deck:resolve 容错通过,直接格式化原样保留。
#[test]
fn resolve_without_inheritance_parts_is_safe() {
    // 复用 parse.rs 的最小合成思路:仅 slide,无 layout/master/theme。
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default();
        let slide = slide_with(
            r#"<p:sp><p:spPr/><p:txBody><a:bodyPr/>
              <a:p><a:r><a:rPr sz="3200" b="1"/><a:t>lone</a:t></a:r></a:p>
            </p:txBody></p:sp>"#,
            "",
        );
        for (name, body) in [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#.to_string(),
            ),
            (
                "ppt/presentation.xml",
                format!(
                    r#"<?xml version="1.0"?><p:presentation {XMLNS}>
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
  <p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
                ),
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"#.to_string(),
            ),
            ("ppt/slides/slide1.xml", slide),
        ] {
            zip.start_file(name, opts).expect("start_file");
            zip.write_all(body.as_bytes()).expect("write");
        }
        zip.finish().expect("finish zip");
    }
    let parsed = parse_bytes(&buf.into_inner()).expect("parse minimal");
    let resolved = resolve(&parsed);
    let run = &as_text_box(&resolved.slides[0].shapes[0]).paragraphs[0].runs[0];
    assert_eq!(run.size_pt, 32.0);
    assert!(run.bold);
    assert_eq!(run.color.rgb, [0x00, 0x00, 0x00], "链上全缺兜底黑");
}
