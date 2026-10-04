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
        <a:ln w="19050"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:headEnd type="oval" w="sm" len="lg"/><a:tailEnd type="stealth"/></a:ln>
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
    build_deck_extra(slide_xml, presentation_extra, layout_xml, master_xml, &[])
}

/// 同 [`build_deck_parts`],另附额外部件(如 `ppt/tableStyles.xml`)。
fn build_deck_extra(
    slide_xml: &str,
    presentation_extra: &str,
    layout_xml: &str,
    master_xml: &str,
    extra_parts: &[(&str, &str)],
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
    // 同名额外部件替换基础部件(如自带图表关系的 slide rels)。
    let parts: Vec<(&str, String)> = parts
        .into_iter()
        .filter(|(n, _)| !extra_parts.iter().any(|(en, _)| en == n))
        .chain(extra_parts.iter().map(|(n, b)| (*n, (*b).to_string())))
        .collect();
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

/// `spc` / `baseline` / `cap` 与其它 run 属性一样沿继承链逐属性合并:master
/// `bodyStyle` 给 spc + cap,layout 占位符 `lstStyle` 给 baseline;slide run 未设则继承,
/// 显式设置则逐属性覆盖;链上全缺为 0 / 0 / `Caps::None`。
#[test]
fn spc_baseline_cap_inherit_through_chain() {
    use ppt_core::style::Caps;
    let master = master1().replace(
        r#"<a:defRPr sz="2800"/>"#,
        r#"<a:defRPr sz="2800" spc="200" cap="small"/>"#,
    );
    let layout = layout1().replace(
        "<a:lstStyle/>",
        r#"<a:lstStyle><a:lvl1pPr><a:defRPr baseline="30000"/></a:lvl1pPr></a:lstStyle>"#,
    );
    let slide_xml = slide_with(
        r#"<p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="C"/><p:cNvSpPr/><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/>
          <a:p>
            <a:r><a:t>inherited</a:t></a:r>
            <a:r><a:rPr spc="-100" baseline="0" cap="none"/><a:t>direct</a:t></a:r>
            <a:r><a:rPr spc="50"/><a:t>partial</a:t></a:r>
          </a:p>
        </p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="4" name="T"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm></p:spPr>
        <p:txBody><a:bodyPr/><a:p><a:r><a:t>none</a:t></a:r></a:p></p:txBody>
      </p:sp>"#,
        "",
    );
    let slide = resolve_slide_parts(&slide_xml, &layout, &master);
    let runs = &as_text_box(&slide.shapes[0]).paragraphs[0].runs;
    assert_eq!(runs[0].char_spacing_pt, 2.0, "master bodyStyle spc=200");
    assert_eq!(runs[0].baseline, 0.30, "layout lstStyle baseline=30000");
    assert_eq!(runs[0].cap, Caps::Small, "master bodyStyle cap=small");

    assert_eq!(runs[1].char_spacing_pt, -1.0, "run 显式负间距覆盖");
    assert_eq!(runs[1].baseline, 0.0, "run 显式 baseline=0 覆盖上标");
    assert_eq!(runs[1].cap, Caps::None, "run 显式 cap=none 覆盖");

    assert_eq!(runs[2].char_spacing_pt, 0.5, "只覆盖 spc");
    assert_eq!(runs[2].baseline, 0.30, "未指定属性仍继承");
    assert_eq!(runs[2].cap, Caps::Small);

    let plain = &as_text_box(&slide.shapes[1]).paragraphs[0].runs[0];
    assert_eq!(plain.char_spacing_pt, 0.0);
    assert_eq!(plain.baseline, 0.0);
    assert_eq!(plain.cap, Caps::None);
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

/// 线端装饰沿描边同一路径继承:显式 `a:ln` 的 headEnd / tailEnd 逐项获胜,缺失时经
/// `lnRef` 取主题 lnStyleLst 档(第 3 档带 oval 头 + stealth 尾)。
#[test]
fn line_ends_inherit_through_ln_ref() {
    use ppt_core::model::{LineEndKind, LineEndSize};
    let conn = |ln: &str| {
        format!(
            r#"<p:cxnSp>
        <p:nvCxnSpPr><p:cNvPr id="4" name="Conn"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm>
          <a:prstGeom prst="line"/>{ln}</p:spPr>
        <p:style><a:lnRef idx="3"><a:schemeClr val="accent1"/></a:lnRef></p:style>
      </p:cxnSp>"#
        )
    };
    let inner = [
        conn(""),
        conn(r#"<a:ln w="25400"><a:tailEnd type="none"/></a:ln>"#),
        conn(r#"<a:ln><a:headEnd type="triangle" w="lg"/></a:ln>"#),
    ]
    .concat();
    let slide = resolve_slide(&slide_with(&inner, ""), "");
    let ends: Vec<_> = slide
        .shapes
        .iter()
        .map(|sh| {
            let ResolvedShape::Connector(c) = sh else {
                panic!("expected connector");
            };
            let s = c.stroke.as_ref().expect("stroke");
            (
                s.head_end.clone().expect("head"),
                s.tail_end.clone().expect("tail"),
            )
        })
        .collect();
    // 1) 全走主题。
    assert_eq!(ends[0].0.kind, LineEndKind::Oval);
    assert_eq!(
        (ends[0].0.width, ends[0].0.length),
        (LineEndSize::Small, LineEndSize::Large)
    );
    assert_eq!(ends[0].1.kind, LineEndKind::Stealth);
    // 2) 显式 tailEnd none 压过主题 stealth;head 仍走主题。
    assert_eq!(ends[1].0.kind, LineEndKind::Oval);
    assert_eq!(ends[1].1.kind, LineEndKind::None);
    // 3) 显式 headEnd 获胜,tail 走主题。
    assert_eq!(ends[2].0.kind, LineEndKind::Triangle);
    assert_eq!(ends[2].0.width, LineEndSize::Large);
    assert_eq!(ends[2].1.kind, LineEndKind::Stealth);
}

/// custGeom 仅经 `p:style` fillRef 着色:终态为带主题填充的自选图形(渲染侧降级画包围盒)。
#[test]
fn style_painted_custom_geometry_resolves_fill() {
    let slide = resolve_slide(
        &slide_with(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="5" name="Freeform"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm><a:custGeom/></p:spPr>
        <p:style><a:lnRef idx="0"/><a:fillRef idx="1"><a:schemeClr val="accent2"/></a:fillRef></p:style>
      </p:sp>"#,
            "",
        ),
        "",
    );
    let ResolvedShape::Auto(a) = &slide.shapes[0] else {
        panic!("expected autoshape");
    };
    assert!(a.custom_geometry);
    assert_rgb_within(
        a.fill.expect("fill from fillRef").color().rgb,
        [0xED, 0x7D, 0x31],
        "fillRef idx=1 phClr=accent2",
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

// ---- master / layout 非占位符形状继承(showMasterSp)--------------------------

/// master:title 占位符(模板,不画)+ accent1 装饰矩形 + 无字号的页脚文本框
/// (字号 / 字体应来自 master `otherStyle`)。
fn master_with_graphics() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster {XMLNS}>
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Title Placeholder 1"/><p:cNvSpPr/>
          <p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="838200" y="365125"/><a:ext cx="7772400" cy="1325563"/></a:xfrm></p:spPr>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Click to edit Master title style</a:t></a:r></a:p></p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="4" name="Bar"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="9144000" cy="254000"/></a:xfrm>
          <a:prstGeom prst="rect"/><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></p:spPr>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="5" name="Footer Text"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="254000" y="6350000"/><a:ext cx="3000000" cy="300000"/></a:xfrm></p:spPr>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>ACME Confidential</a:t></a:r></a:p></p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
  <p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2"
            accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6"
            hlink="hlink" folHlink="folHlink"/>
  <p:txStyles>
    <p:titleStyle><a:lvl1pPr><a:defRPr sz="4400"/></a:lvl1pPr></p:titleStyle>
    <p:bodyStyle><a:lvl1pPr><a:defRPr sz="2800"/></a:lvl1pPr></p:bodyStyle>
    <p:otherStyle><a:lvl1pPr><a:defRPr sz="1100"><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl1pPr></p:otherStyle>
  </p:txStyles>
</p:sldMaster>"#
    )
}

/// layout:body 占位符(模板,不画)+ 一条页脚分隔线;`root_attrs` 注入根元素属性。
fn layout_with_line(root_attrs: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout {XMLNS}{root_attrs}>
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Content 2"/><p:cNvSpPr/>
          <p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>layout body prompt</a:t></a:r></a:p></p:txBody>
      </p:sp>
      <p:cxnSp>
        <p:nvCxnSpPr><p:cNvPr id="6" name="Rule"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr>
        <p:spPr><a:xfrm><a:off x="254000" y="6300000"/><a:ext cx="8636000" cy="0"/></a:xfrm>
          <a:prstGeom prst="line"/><a:ln w="12700"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:ln></p:spPr>
      </p:cxnSp>
    </p:spTree>
  </p:cSld>
</p:sldLayout>"#
    )
}

fn slide_body_with_root(root_attrs: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld {XMLNS}{root_attrs}>
  <p:cSld><p:spTree>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="3" name="Content 2"/><p:cNvSpPr/><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr>
      <p:spPr/>
      <p:txBody><a:bodyPr/><a:p><a:r><a:t>slide body</a:t></a:r></a:p></p:txBody>
    </p:sp>
  </p:spTree></p:cSld>
</p:sld>"#
    )
}

fn inherited_kinds(slide: &ResolvedSlide) -> Vec<&'static str> {
    slide
        .inherited_shapes
        .iter()
        .map(|s| match s {
            ResolvedShape::Auto(_) => "auto",
            ResolvedShape::TextBox(_) => "text",
            ResolvedShape::Connector(_) => "connector",
            _ => "other",
        })
        .collect()
}

/// 非占位符继承形状:master 层在前(文档顺序)、layout 层在后;占位符模板不进;
/// slide 自身形状仍只在 `shapes`。
#[test]
fn master_and_layout_graphics_are_inherited_in_order() {
    let slide = resolve_slide_parts(
        &slide_body_with_root(""),
        &layout_with_line(""),
        &master_with_graphics(),
    );
    assert_eq!(inherited_kinds(&slide), ["auto", "text", "connector"]);
    assert_eq!(slide.shapes.len(), 1, "slide.shapes 只含 slide 自身形状");
    let ResolvedShape::Auto(bar) = &slide.inherited_shapes[0] else {
        unreachable!()
    };
    let fill = bar.fill.expect("bar fill").color();
    assert_eq!(
        fill.rgb,
        [0x44, 0x72, 0xC4],
        "accent1 经 clrMap + theme 终端化"
    );
    let ResolvedShape::Connector(rule) = &slide.inherited_shapes[2] else {
        unreachable!()
    };
    assert_eq!(
        rule.stroke.as_ref().and_then(|s| s.color).map(|c| c.rgb),
        Some([255, 0, 0])
    );
}

/// 母版文本框文字走 master `otherStyle`(字号 11pt、`+mn-lt` 展开为主题 minor 字体)。
#[test]
fn master_text_uses_master_other_style() {
    let slide = resolve_slide_parts(
        &slide_body_with_root(""),
        &layout_with_line(""),
        &master_with_graphics(),
    );
    let run = &as_text_box(&slide.inherited_shapes[1]).paragraphs[0].runs[0];
    assert_eq!(run.text, "ACME Confidential");
    assert_eq!(run.size_pt, 11.0);
    assert_eq!(run.font.as_deref(), Some("Calibri"));
}

/// slide `showMasterSp="0"`(隐藏背景图形):master 与 layout 图形都不画。
#[test]
fn slide_show_master_sp_false_hides_all_inherited() {
    let slide = resolve_slide_parts(
        &slide_body_with_root(r#" showMasterSp="0""#),
        &layout_with_line(""),
        &master_with_graphics(),
    );
    assert!(
        slide.inherited_shapes.is_empty(),
        "{:?}",
        inherited_kinds(&slide)
    );
    assert_eq!(slide.shapes.len(), 1);
}

/// layout `showMasterSp="0"`:只隐藏 master 图形,layout 自身图形照画。
#[test]
fn layout_show_master_sp_false_hides_master_only() {
    let slide = resolve_slide_parts(
        &slide_body_with_root(""),
        &layout_with_line(r#" showMasterSp="0""#),
        &master_with_graphics(),
    );
    assert_eq!(inherited_kinds(&slide), ["connector"]);
}

/// 显式 `showMasterSp="1"` 与缺省同义。
#[test]
fn show_master_sp_true_is_default() {
    let slide = resolve_slide_parts(
        &slide_body_with_root(r#" showMasterSp="1""#),
        &layout_with_line(r#" showMasterSp="true""#),
        &master_with_graphics(),
    );
    assert_eq!(inherited_kinds(&slide), ["auto", "text", "connector"]);
}

/// 只有占位符的 master / layout(既有 fixture 的形态)→ 无继承形状。
#[test]
fn placeholder_only_parts_inherit_nothing() {
    assert!(resolve_default().inherited_shapes.is_empty());
}

// ---- 表格样式(`ppt/tableStyles.xml`)----------------------------------------

/// 测试用表格样式:wholeTbl(accent1 tint20 填充 + 六向边框 + tx2 文字色)、
/// band1H / band1V / firstCol / lastCol / lastRow / firstRow 各带可辨识的填充;
/// firstCol 左边框显式 `noFill`;`seCell` 角单元格(不支持,应被跳过)。
/// `{GRAD}`:wholeTbl 渐变填充(不支持,应被跳过)+ 纯色上边框。
const TABLE_STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:tblStyleLst xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" def="{TEST-STYLE}">
  <a:tblStyle styleId="{TEST-STYLE}" styleName="Test Style">
    <a:wholeTbl>
      <a:tcTxStyle><a:fontRef idx="minor"><a:prstClr val="black"/></a:fontRef><a:schemeClr val="tx2"/></a:tcTxStyle>
      <a:tcStyle>
        <a:tcBdr>
          <a:left><a:ln w="12700"><a:solidFill><a:srgbClr val="111111"/></a:solidFill></a:ln></a:left>
          <a:right><a:ln w="12700"><a:solidFill><a:srgbClr val="222222"/></a:solidFill></a:ln></a:right>
          <a:top><a:ln w="12700"><a:solidFill><a:srgbClr val="333333"/></a:solidFill></a:ln></a:top>
          <a:bottom><a:ln w="12700"><a:solidFill><a:srgbClr val="444444"/></a:solidFill></a:ln></a:bottom>
          <a:insideH><a:ln w="6350"><a:solidFill><a:srgbClr val="555555"/></a:solidFill></a:ln></a:insideH>
          <a:insideV><a:ln w="6350"><a:solidFill><a:srgbClr val="666666"/></a:solidFill></a:ln></a:insideV>
        </a:tcBdr>
        <a:fill><a:solidFill><a:schemeClr val="accent1"><a:tint val="20000"/></a:schemeClr></a:solidFill></a:fill>
      </a:tcStyle>
    </a:wholeTbl>
    <a:band1H><a:tcStyle><a:tcBdr/><a:fill><a:solidFill><a:srgbClr val="B1B1B1"/></a:solidFill></a:fill></a:tcStyle></a:band1H>
    <a:band2H><a:tcStyle><a:tcBdr/></a:tcStyle></a:band2H>
    <a:band1V><a:tcStyle><a:tcBdr/><a:fill><a:solidFill><a:srgbClr val="C1C1C1"/></a:solidFill></a:fill></a:tcStyle></a:band1V>
    <a:band2V><a:tcStyle><a:tcBdr/></a:tcStyle></a:band2V>
    <a:lastCol><a:tcTxStyle b="on"/><a:tcStyle><a:tcBdr/><a:fill><a:solidFill><a:srgbClr val="D2D2D2"/></a:solidFill></a:fill></a:tcStyle></a:lastCol>
    <a:firstCol>
      <a:tcTxStyle b="on"/>
      <a:tcStyle>
        <a:tcBdr><a:left><a:ln w="12700"><a:noFill/></a:ln></a:left></a:tcBdr>
        <a:fill><a:solidFill><a:srgbClr val="F1F1F1"/></a:solidFill></a:fill>
      </a:tcStyle>
    </a:firstCol>
    <a:lastRow>
      <a:tcTxStyle b="on"/>
      <a:tcStyle>
        <a:tcBdr><a:top><a:ln w="38100"><a:solidFill><a:srgbClr val="E0E0E0"/></a:solidFill></a:ln></a:top></a:tcBdr>
        <a:fill><a:solidFill><a:srgbClr val="E1E1E1"/></a:solidFill></a:fill>
      </a:tcStyle>
    </a:lastRow>
    <a:seCell><a:tcStyle><a:fill><a:solidFill><a:srgbClr val="0F0F0F"/></a:solidFill></a:fill></a:tcStyle></a:seCell>
    <a:firstRow>
      <a:tcTxStyle b="on"><a:fontRef idx="minor"><a:prstClr val="black"/></a:fontRef><a:schemeClr val="lt1"/></a:tcTxStyle>
      <a:tcStyle>
        <a:tcBdr><a:bottom><a:ln w="38100"><a:solidFill><a:schemeClr val="lt1"/></a:solidFill></a:ln></a:bottom></a:tcBdr>
        <a:fill><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></a:fill>
      </a:tcStyle>
    </a:firstRow>
  </a:tblStyle>
  <a:tblStyle styleId="{GRAD}" styleName="Gradient">
    <a:wholeTbl>
      <a:tcStyle>
        <a:tcBdr><a:top><a:ln w="12700"><a:solidFill><a:srgbClr val="777777"/></a:solidFill></a:ln></a:top></a:tcBdr>
        <a:fill><a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="FF0000"/></a:gs></a:gsLst></a:gradFill></a:fill>
      </a:tcStyle>
    </a:wholeTbl>
  </a:tblStyle>
</a:tblStyleLst>"#;

/// `nrows × ncols` 的纯文本单元格行(无 `tcPr`),文字 `r{行}c{列}`。
fn plain_rows(nrows: usize, ncols: usize) -> String {
    (0..nrows)
        .map(|r| {
            let cells: String = (0..ncols)
                .map(|c| format!(r#"<a:tc><a:txBody><a:p><a:r><a:t>r{r}c{c}</a:t></a:r></a:p></a:txBody></a:tc>"#))
                .collect();
            format!(r#"<a:tr h="100">{cells}</a:tr>"#)
        })
        .collect()
}

/// 一张带给定 `a:tblPr`(原样)与行 XML 的表格 slide。
fn styled_table_slide(tbl_pr: &str, ncols: usize, rows_xml: &str) -> String {
    let grid: String = (0..ncols).map(|_| r#"<a:gridCol w="100"/>"#).collect();
    slide_with(
        &format!(
            r#"<p:graphicFrame>
        <p:xfrm><a:off x="0" y="0"/><a:ext cx="1000" cy="1000"/></p:xfrm>
        <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
          <a:tbl>{tbl_pr}<a:tblGrid>{grid}</a:tblGrid>{rows_xml}</a:tbl>
        </a:graphicData></a:graphic>
      </p:graphicFrame>"#
        ),
        "",
    )
}

/// 解析 + 继承链解析,返回唯一表格;`table_styles` 为 `ppt/tableStyles.xml` 内容(`None` = 部件缺失)。
fn resolve_styled_table(
    slide_xml: &str,
    table_styles: Option<&str>,
) -> ppt_core::resolved::ResolvedTable {
    let extra: Vec<(&str, &str)> = table_styles
        .map(|x| vec![("ppt/tableStyles.xml", x)])
        .unwrap_or_default();
    let deck = build_deck_extra(slide_xml, "", &layout1(), &master1(), &extra);
    let parsed = parse_bytes(&deck).expect("parse deck");
    let slide = resolve(&parsed)
        .slides
        .into_iter()
        .next()
        .expect("one slide");
    match slide.shapes.into_iter().next() {
        Some(ResolvedShape::Table(t)) => t,
        other => panic!("expected table, got {other:?}"),
    }
}

/// wholeTbl 填充色:accent1(4472C4)tint 20%(变换数学已由金标测试覆盖,这里只验接线)。
fn whole_tbl_fill() -> [u8; 3] {
    use ppt_core::color::{apply_transforms, ColorTransform};
    apply_transforms([0x44, 0x72, 0xC4], &[ColorTransform::Tint(20_000)]).rgb
}

fn cell_fill(t: &ppt_core::resolved::ResolvedTable, r: usize, c: usize) -> Option<[u8; 3]> {
    t.rows[r].cells[c].fill.map(|f| f.rgb)
}

fn edge_rgb(s: &Option<ppt_core::resolved::ResolvedStroke>) -> Option<[u8; 3]> {
    s.as_ref().and_then(|s| s.color).map(|c| c.rgb)
}

fn first_run(t: &ppt_core::resolved::ResolvedTable, r: usize, c: usize) -> &ResolvedRunAlias {
    &t.rows[r].cells[c].paragraphs[0].runs[0]
}

type ResolvedRunAlias = ppt_core::resolved::ResolvedRun;

/// 纯 wholeTbl(无开关):填充 / 文字色落到每格;外沿取 left/right/top/bottom,
/// 格间取 insideH / insideV。
#[test]
fn table_style_whole_tbl_applies_fill_borders_and_text() {
    let xml = styled_table_slide(
        r#"<a:tblPr><a:tableStyleId>{TEST-STYLE}</a:tableStyleId></a:tblPr>"#,
        2,
        &plain_rows(2, 2),
    );
    let t = resolve_styled_table(&xml, Some(TABLE_STYLES));
    assert!(t.style_resolved, "styleId 找到即标记已解析");
    for r in 0..2 {
        for c in 0..2 {
            assert_eq!(
                cell_fill(&t, r, c),
                Some(whole_tbl_fill()),
                "({r},{c}) wholeTbl 填充"
            );
            let run = first_run(&t, r, c);
            assert_eq!(run.color.rgb, [0x44, 0x54, 0x6A], "tx2 -> dk2 文字色");
            assert!(!run.bold);
        }
    }
    let b00 = &t.rows[0].cells[0].borders;
    assert_eq!(edge_rgb(&b00.left), Some([0x11; 3]), "外沿左");
    assert_eq!(edge_rgb(&b00.top), Some([0x33; 3]), "外沿上");
    assert_eq!(edge_rgb(&b00.right), Some([0x66; 3]), "格间竖线 insideV");
    assert_eq!(edge_rgb(&b00.bottom), Some([0x55; 3]), "格间横线 insideH");
    assert_eq!(b00.left.as_ref().and_then(|s| s.width_emu), Some(12_700));
    let b11 = &t.rows[1].cells[1].borders;
    assert_eq!(edge_rgb(&b11.right), Some([0x22; 3]), "外沿右");
    assert_eq!(edge_rgb(&b11.bottom), Some([0x44; 3]), "外沿下");
    assert_eq!(edge_rgb(&b11.left), Some([0x66; 3]));
    assert_eq!(edge_rgb(&b11.top), Some([0x55; 3]));
}

/// firstRow + bandRow:表头行不计入行带计数——第 1 行是 band1H,第 2 行 band2H(无填充 →
/// 露出 wholeTbl),第 3 行又是 band1H;表头取 firstRow 填充 / 白字 / 粗体 / 下边框。
#[test]
fn table_style_first_row_and_band_row_zebra_skip_header() {
    let xml = styled_table_slide(
        r#"<a:tblPr firstRow="1" bandRow="1"><a:tableStyleId>{TEST-STYLE}</a:tableStyleId></a:tblPr>"#,
        2,
        &plain_rows(4, 2),
    );
    let t = resolve_styled_table(&xml, Some(TABLE_STYLES));
    for c in 0..2 {
        assert_eq!(
            cell_fill(&t, 0, c),
            Some([0x44, 0x72, 0xC4]),
            "表头 accent1"
        );
        let run = first_run(&t, 0, c);
        assert_eq!(run.color.rgb, [0xFF; 3], "表头 lt1 文字");
        assert!(run.bold, "表头 b=on");
        let b = &t.rows[0].cells[c].borders;
        assert_eq!(edge_rgb(&b.bottom), Some([0xFF; 3]), "firstRow 下边框");
        assert_eq!(b.bottom.as_ref().and_then(|s| s.width_emu), Some(38_100));
        assert_eq!(
            edge_rgb(&b.top),
            Some([0x33; 3]),
            "firstRow 未指定上边 → wholeTbl 外沿"
        );
        assert_eq!(cell_fill(&t, 1, c), Some([0xB1; 3]), "第 1 行 band1H");
        assert_eq!(
            cell_fill(&t, 2, c),
            Some(whole_tbl_fill()),
            "第 2 行 band2H 无填充"
        );
        assert_eq!(cell_fill(&t, 3, c), Some([0xB1; 3]), "第 3 行 band1H");
        for r in 1..4 {
            let run = first_run(&t, r, c);
            assert_eq!(run.color.rgb, [0x44, 0x54, 0x6A]);
            assert!(!run.bold, "({r},{c}) 非表头不加粗");
        }
    }
    // 行带区域是单行:band1H 未给边框 → 仍是 wholeTbl 的格间横线。
    assert_eq!(edge_rgb(&t.rows[1].cells[0].borders.top), Some([0x55; 3]));
}

/// bandCol:列带交替(band1V / band2V);未开 bandRow 时行带不生效。
#[test]
fn table_style_band_col_alternates_columns() {
    let xml = styled_table_slide(
        r#"<a:tblPr bandCol="1"><a:tableStyleId>{TEST-STYLE}</a:tableStyleId></a:tblPr>"#,
        3,
        &plain_rows(2, 3),
    );
    let t = resolve_styled_table(&xml, Some(TABLE_STYLES));
    for r in 0..2 {
        assert_eq!(cell_fill(&t, r, 0), Some([0xC1; 3]), "band1V");
        assert_eq!(cell_fill(&t, r, 1), Some(whole_tbl_fill()), "band2V 无填充");
        assert_eq!(cell_fill(&t, r, 2), Some([0xC1; 3]), "band1V");
    }
}

/// 优先级:band < firstCol < lastRow;firstCol 的显式 noFill 左边框压制 wholeTbl 外沿;
/// lastRow 上边框(区域外沿)覆盖 wholeTbl 的格间横线;角单元格 `seCell` 被跳过。
#[test]
fn table_style_first_col_last_row_priority() {
    let xml = styled_table_slide(
        r#"<a:tblPr firstCol="1" lastRow="1" bandRow="1"><a:tableStyleId>{TEST-STYLE}</a:tableStyleId></a:tblPr>"#,
        2,
        &plain_rows(3, 2),
    );
    let t = resolve_styled_table(&xml, Some(TABLE_STYLES));
    assert_eq!(cell_fill(&t, 0, 0), Some([0xF1; 3]), "firstCol 胜过 band1H");
    assert_eq!(
        cell_fill(&t, 0, 1),
        Some([0xB1; 3]),
        "无 firstRow:第 0 行即 band1H"
    );
    assert_eq!(cell_fill(&t, 1, 0), Some([0xF1; 3]));
    assert_eq!(cell_fill(&t, 1, 1), Some(whole_tbl_fill()), "band2H 无填充");
    assert_eq!(
        cell_fill(&t, 2, 0),
        Some([0xE1; 3]),
        "lastRow 胜过 firstCol"
    );
    assert_eq!(
        cell_fill(&t, 2, 1),
        Some([0xE1; 3]),
        "lastRow 胜过 seCell(不支持,跳过)"
    );
    assert!(first_run(&t, 1, 0).bold, "firstCol b=on");
    assert!(!first_run(&t, 1, 1).bold);
    assert!(
        t.rows[0].cells[0].borders.left.is_none(),
        "firstCol 显式 noFill 左边框"
    );
    assert!(
        t.rows[2].cells[0].borders.left.is_none(),
        "lastRow 未指定左边 → 保留 firstCol 的无线"
    );
    let top = &t.rows[2].cells[1].borders.top;
    assert_eq!(edge_rgb(top), Some([0xE0; 3]), "lastRow 上边框");
    assert_eq!(top.as_ref().and_then(|s| s.width_emu), Some(38_100));
}

/// 显式 `tcPr` / run 属性永远胜出:solidFill / noFill / lnB / rPr 颜色与粗体。
#[test]
fn explicit_tcpr_overrides_table_style() {
    let rows = r#"<a:tr h="100">
        <a:tc>
          <a:txBody><a:p><a:r><a:rPr b="0"><a:solidFill><a:srgbClr val="0000FF"/></a:solidFill></a:rPr><a:t>x</a:t></a:r></a:p></a:txBody>
          <a:tcPr>
            <a:lnB w="9525"><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></a:lnB>
            <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
          </a:tcPr>
        </a:tc>
        <a:tc><a:txBody><a:p><a:r><a:t>y</a:t></a:r></a:p></a:txBody><a:tcPr><a:noFill/></a:tcPr></a:tc>
      </a:tr>"#;
    let xml = styled_table_slide(
        r#"<a:tblPr firstRow="1"><a:tableStyleId>{TEST-STYLE}</a:tableStyleId></a:tblPr>"#,
        2,
        &format!("{rows}{}", plain_rows(1, 2)),
    );
    let t = resolve_styled_table(&xml, Some(TABLE_STYLES));
    assert_eq!(
        cell_fill(&t, 0, 0),
        Some([0xFF, 0x00, 0x00]),
        "显式 solidFill 胜"
    );
    assert_eq!(cell_fill(&t, 0, 1), None, "显式 noFill 压制样式填充");
    let b = &t.rows[0].cells[0].borders;
    assert_eq!(edge_rgb(&b.bottom), Some([0x00, 0xFF, 0x00]), "显式 lnB 胜");
    assert_eq!(b.bottom.as_ref().and_then(|s| s.width_emu), Some(9_525));
    assert_eq!(edge_rgb(&b.left), Some([0x11; 3]), "未显式的边仍取样式");
    let run = first_run(&t, 0, 0);
    assert!(!run.bold, "rPr b=0 胜过 firstRow b=on");
    assert_eq!(run.color.rgb, [0x00, 0x00, 0xFF], "rPr 颜色胜过样式文字色");
    let run = first_run(&t, 0, 1);
    assert!(run.bold, "无显式 run 属性 → 样式粗体");
    assert_eq!(run.color.rgb, [0xFF; 3]);
    assert_eq!(cell_fill(&t, 1, 0), Some(whole_tbl_fill()));
}

/// 显式无边框 `a:lnX > a:noFill`(带 / 不带线宽)= 该边不画,且作为显式属性压制表格样式的边;
/// 无样式表格里同样不画(不能退化成只有线宽的缺省黑线)。
#[test]
fn explicit_no_fill_cell_border_is_not_drawn() {
    let rows = r#"<a:tr h="100">
        <a:tc>
          <a:txBody><a:p><a:r><a:t>x</a:t></a:r></a:p></a:txBody>
          <a:tcPr>
            <a:lnL w="12700"><a:noFill/></a:lnL>
            <a:lnR w="12700" cap="flat"><a:noFill/></a:lnR>
            <a:lnT><a:noFill/></a:lnT>
            <a:lnB w="12700"><a:noFill/><a:prstDash val="solid"/></a:lnB>
          </a:tcPr>
        </a:tc>
      </a:tr>"#;
    for tbl_pr in [
        r#"<a:tblPr><a:tableStyleId>{TEST-STYLE}</a:tableStyleId></a:tblPr>"#,
        r#"<a:tblPr/>"#,
    ] {
        let xml = styled_table_slide(tbl_pr, 1, &format!("{rows}{}", plain_rows(1, 1)));
        let t = resolve_styled_table(&xml, Some(TABLE_STYLES));
        let b = &t.rows[0].cells[0].borders;
        assert!(
            b.left.is_none(),
            "{tbl_pr}: 显式 noFill 左边不画: {:?}",
            b.left
        );
        assert!(b.right.is_none(), "{tbl_pr}: 显式 noFill 右边不画");
        assert!(b.top.is_none(), "{tbl_pr}: 显式 noFill 上边不画");
        assert!(b.bottom.is_none(), "{tbl_pr}: 显式 noFill 下边不画");
    }
    // 对照:同表下一行未显式的边仍取样式(wholeTbl 左外沿 111111)。
    let xml = styled_table_slide(
        r#"<a:tblPr><a:tableStyleId>{TEST-STYLE}</a:tableStyleId></a:tblPr>"#,
        1,
        &format!("{rows}{}", plain_rows(1, 1)),
    );
    let t = resolve_styled_table(&xml, Some(TABLE_STYLES));
    assert_eq!(edge_rgb(&t.rows[1].cells[0].borders.left), Some([0x11; 3]));
}

/// styleId 找不到 / `tableStyles.xml` 缺失:退回旧行为(只用显式属性),标记未解析。
#[test]
fn table_style_missing_falls_back_to_explicit_only() {
    let rows = r#"<a:tr h="100">
        <a:tc><a:txBody><a:p><a:r><a:t>x</a:t></a:r></a:p></a:txBody>
          <a:tcPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:tcPr></a:tc>
        <a:tc><a:txBody><a:p><a:r><a:t>y</a:t></a:r></a:p></a:txBody></a:tc>
      </a:tr>"#;
    for (id, styles) in [("{NOPE}", Some(TABLE_STYLES)), ("{TEST-STYLE}", None)] {
        let xml = styled_table_slide(
            &format!(
                r#"<a:tblPr firstRow="1" bandRow="1"><a:tableStyleId>{id}</a:tableStyleId></a:tblPr>"#
            ),
            2,
            rows,
        );
        let t = resolve_styled_table(&xml, styles);
        assert_eq!(t.table_style_id.as_deref(), Some(id));
        assert!(!t.style_resolved, "{id}: 未解析");
        assert_eq!(
            cell_fill(&t, 0, 0),
            Some([0xFF, 0x00, 0x00]),
            "显式填充保留"
        );
        assert_eq!(cell_fill(&t, 0, 1), None, "无样式填充");
        let b = &t.rows[0].cells[1].borders;
        assert!(b.left.is_none() && b.right.is_none() && b.top.is_none() && b.bottom.is_none());
        let run = first_run(&t, 0, 1);
        assert_eq!(run.color.rgb, [0, 0, 0], "文字色回到旧链兜底");
        assert!(!run.bold);
    }
}

/// 不支持的填充(渐变)跳过该项,其余部分(边框)照常生效。
#[test]
fn table_style_unsupported_gradient_fill_is_skipped() {
    let xml = styled_table_slide(
        r#"<a:tblPr><a:tableStyleId>{GRAD}</a:tableStyleId></a:tblPr>"#,
        1,
        &plain_rows(1, 1),
    );
    let t = resolve_styled_table(&xml, Some(TABLE_STYLES));
    assert!(t.style_resolved);
    assert_eq!(cell_fill(&t, 0, 0), None, "渐变填充跳过");
    assert_eq!(edge_rgb(&t.rows[0].cells[0].borders.top), Some([0x77; 3]));
}

/// 畸形 `tableStyles.xml`(截断 / 非 XML 垃圾)绝不 panic;垃圾输入找不到样式 → 降级。
#[test]
fn malformed_table_styles_never_panics() {
    let xml = styled_table_slide(
        r#"<a:tblPr firstRow="1"><a:tableStyleId>{TEST-STYLE}</a:tableStyleId></a:tblPr>"#,
        2,
        &plain_rows(2, 2),
    );
    let truncated = &TABLE_STYLES[..TABLE_STYLES.len() / 2];
    let t = resolve_styled_table(&xml, Some(truncated));
    assert_eq!(t.rows.len(), 2, "截断部件:表格照常解析");
    let garbage = "\u{0}<<<not xml </a:tblStyle> <a:tblStyle styleId=";
    let t = resolve_styled_table(&xml, Some(garbage));
    assert!(!t.style_resolved, "垃圾部件:找不到样式");
    assert_eq!(cell_fill(&t, 0, 0), None);
}

// ---- 图表配色(主题 accent1..6)------------------------------------------------

/// `ResolvedSlide.accents`:主题 accent1..6 经 clrMap 重映射后的终端色(图表系列配色用)。
#[test]
fn slide_accents_follow_theme_through_clr_map() {
    let slide = resolve_default();
    assert_eq!(
        slide.accents,
        [
            [0x44, 0x72, 0xC4],
            [0xED, 0x7D, 0x31],
            [0xA5, 0xA5, 0xA5],
            [0xFF, 0xC0, 0x00],
            [0x5B, 0x9B, 0xD5],
            [0x70, 0xAD, 0x47],
        ]
    );
    let master = master1().replace(r#"accent1="accent1""#, r#"accent1="accent6""#);
    assert_ne!(master, master1(), "fixture 应含 accent1 映射");
    let slide = resolve_slide_parts(&slide_default(), &layout1(), &master);
    assert_eq!(
        slide.accents[0],
        [0x70, 0xAD, 0x47],
        "clrMap accent1 → accent6"
    );
    assert_eq!(slide.accents[1], [0xED, 0x7D, 0x31]);
}

// ---- 自选图形 / 连接线显式无轮廓(a:ln > a:noFill)--------------------------

/// 一个带 `lnRef idx=2`(主题线 w=12700)的矩形,`ln` 是 spPr 里的 `a:ln` 片段。
fn styled_rect(ln: &str) -> String {
    format!(
        r#"<p:sp>
        <p:spPr><a:prstGeom prst="rect"/>{ln}</p:spPr>
        <p:style><a:lnRef idx="2"><a:schemeClr val="accent1"/></a:lnRef></p:style>
      </p:sp>"#
    )
}

fn auto_stroke(xml: &str) -> Option<ppt_core::resolved::ResolvedStroke> {
    match &resolve_slide(&slide_with(xml, ""), "").shapes[0] {
        ResolvedShape::Auto(a) => a.stroke.clone(),
        other => panic!("expected auto shape, got {other:?}"),
    }
}

/// 显式 `a:ln > a:noFill`(带 / 不带线宽)= 不画线,且胜过 `lnRef` 主题线;
/// 无 `a:ln` 时仍走 `lnRef` 继承;显式有色线照常;无 `a:ln` 无样式 = 无描边。
#[test]
fn explicit_no_fill_shape_outline_is_not_drawn() {
    for ln in [
        r#"<a:ln w="25400"><a:noFill/></a:ln>"#,
        r#"<a:ln><a:noFill/></a:ln>"#,
        r#"<a:ln w="25400" cap="flat"><a:noFill/><a:prstDash val="solid"/></a:ln>"#,
    ] {
        assert_eq!(
            auto_stroke(&styled_rect(ln)),
            None,
            "{ln}: 显式无线压制 lnRef"
        );
    }
    // 对照 1:无 a:ln → 继承 lnRef 主题线(不能回归成"全都不画")。
    let inherited = auto_stroke(&styled_rect("")).expect("lnRef 继承线");
    assert_eq!(inherited.width_emu, Some(12_700));
    // 对照 2:显式有色线胜过继承。
    let own = auto_stroke(&styled_rect(
        r#"<a:ln w="38100"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:ln>"#,
    ))
    .expect("显式线");
    assert_eq!(own.width_emu, Some(38_100));
    assert_eq!(own.color.expect("color").rgb, [0xFF, 0x00, 0x00]);
    // 对照 3:无 a:ln 无样式 → 无描边(旧行为)。
    assert_eq!(
        auto_stroke(r#"<p:sp><p:spPr><a:prstGeom prst="rect"/></p:spPr></p:sp>"#),
        None
    );
}

/// 连接线显式 `a:ln > a:noFill`:`stroke` 为空且 `no_line` = true(渲染侧据此不套
/// "无描边 → 缺省黑线"兜底);无 `a:ln` 时 `no_line` = false。
#[test]
fn explicit_no_fill_connector_sets_no_line() {
    let conn = |ln: &str| {
        format!(
            r#"<p:cxnSp>
        <p:nvCxnSpPr><p:cNvPr id="4" name="Conn"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr>
        <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm>
          <a:prstGeom prst="line"/>{ln}</p:spPr>
        <p:style><a:lnRef idx="2"><a:schemeClr val="accent1"/></a:lnRef></p:style>
      </p:cxnSp>"#
        )
    };
    let inner = [conn(r#"<a:ln w="12700"><a:noFill/></a:ln>"#), conn("")].concat();
    let slide = resolve_slide(&slide_with(&inner, ""), "");
    let get = |i: usize| match &slide.shapes[i] {
        ResolvedShape::Connector(c) => c.clone(),
        other => panic!("expected connector, got {other:?}"),
    };
    let (hidden, inherited) = (get(0), get(1));
    assert!(hidden.stroke.is_none() && hidden.no_line, "显式无线");
    assert!(
        inherited.stroke.is_some() && !inherited.no_line,
        "继承 lnRef"
    );
}

// ---- 图表系列色:schemeClr + 变换经主题终端化 --------------------------------

/// 图表 `c:spPr` / `c:dPt` 里的 schemeClr(带 lumMod)在解析阶段落成显式 srgb(经 clrMap +
/// clrScheme + 变换),渲染侧据此直接取色;srgb 原样保留。
#[test]
fn chart_series_scheme_colors_resolve_through_theme() {
    use ppt_core::color::ColorSpec;
    let frame = r#"<p:graphicFrame>
      <p:nvGraphicFramePr><p:cNvPr id="4" name="Chart"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
      <p:xfrm><a:off x="0" y="0"/><a:ext cx="4000000" cy="3000000"/></p:xfrm>
      <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">
        <c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId5"/>
      </a:graphicData></a:graphic></p:graphicFrame>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
  <Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/>
</Relationships>"#;
    let chart = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
        xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea>
      <c:barChart><c:barDir val="col"/>
        <c:ser><c:spPr><a:solidFill><a:schemeClr val="accent1"><a:lumMod val="75000"/></a:schemeClr></a:solidFill></c:spPr>
          <c:dPt><c:idx val="1"/><c:spPr><a:solidFill><a:srgbClr val="123456"/></a:solidFill></c:spPr></c:dPt>
          <c:dPt><c:idx val="2"/><c:spPr><a:solidFill><a:schemeClr val="accent2"/></a:solidFill></c:spPr></c:dPt>
          <c:val><c:numLit><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser>
      </c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
    let xml = build_deck_extra(
        &slide_with(frame, ""),
        "",
        &layout1(),
        &master1(),
        &[
            ("ppt/slides/_rels/slide1.xml.rels", rels),
            ("ppt/charts/chart1.xml", chart),
        ],
    );
    let slide = resolve(&parse_bytes(&xml).expect("parse")).slides.remove(0);
    let ResolvedShape::Placeholder(gp) = &slide.shapes[0] else {
        panic!("expected chart placeholder");
    };
    let s = &gp.chart.as_ref().expect("chart").series[0];
    let rgb = |c: &ColorSpec| match c {
        ColorSpec::Srgb { rgb, transforms } if transforms.is_empty() => *rgb,
        other => panic!("颜色应已终端化为 srgb: {other:?}"),
    };
    assert_rgb_within(
        rgb(s.color.as_ref().expect("color")),
        [0x2F, 0x55, 0x97],
        "accent1 lumMod75",
    );
    assert_eq!(rgb(&s.point_colors[0].1), [0x12, 0x34, 0x56]);
    assert_rgb_within(rgb(&s.point_colors[1].1), [0xED, 0x7D, 0x31], "dPt accent2");
}

// ---- run 级超链接进入终态 IR ---------------------------------------------------

/// 外链(rels `Target`)落到 `ResolvedRun.link`(URL 原文,scheme 过滤在渲染侧);页内跳转 /
/// 无链接为 `None`。
#[test]
fn run_external_hyperlink_reaches_resolved_run() {
    let slide = slide_with(
        r#"<p:sp><p:spPr/><p:txBody><a:bodyPr/><a:p>
          <a:r><a:rPr><a:hlinkClick r:id="rId7"/></a:rPr><a:t>web</a:t></a:r>
          <a:r><a:rPr><a:hlinkClick r:id="rId8" action="ppaction://hlinksldjump"/></a:rPr><a:t>jump</a:t></a:r>
          <a:r><a:t>plain</a:t></a:r>
        </a:p></p:txBody></p:sp>"#,
        "",
    );
    let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
  <Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/a" TargetMode="External"/>
  <Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slide1.xml"/>
</Relationships>"#;
    let xml = build_deck_extra(
        &slide,
        "",
        &layout1(),
        &master1(),
        &[("ppt/slides/_rels/slide1.xml.rels", rels)],
    );
    let resolved = resolve(&parse_bytes(&xml).expect("parse"));
    let runs = &as_text_box(&resolved.slides[0].shapes[0]).paragraphs[0].runs;
    assert_eq!(runs[0].link.as_deref(), Some("https://example.com/a"));
    assert_eq!(runs[1].link, None, "页内跳转不是外链");
    assert_eq!(runs[2].link, None);
}
