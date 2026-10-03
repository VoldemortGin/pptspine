//! fuzz target 与种子生成器共用的最小 `.pptx` 打包帮助函数(现场构造,不落二进制 fixture)。

use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const OD: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

pub const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
  xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;

pub const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
</Types>"#;

pub const PRESENTATION: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
  xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId2"/></p:sldMasterIdLst>
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
  <p:sldSz cx="9144000" cy="6858000" type="screen4x3"/>
</p:presentation>"#;

pub const MASTER: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
  xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="838200" y="365125"/><a:ext cx="7772400" cy="1325563"/></a:xfrm></p:spPr>
      <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Master title</a:t></a:r></a:p></p:txBody></p:sp>
    <p:sp><p:nvSpPr><p:cNvPr id="3" name="Body"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="838200" y="1825625"/><a:ext cx="7772400" cy="4351338"/></a:xfrm></p:spPr>
      <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Master body</a:t></a:r></a:p></p:txBody></p:sp>
  </p:spTree></p:cSld>
  <p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2"
    accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>
  <p:txStyles>
    <p:titleStyle><a:lvl1pPr algn="ctr"><a:buNone/><a:defRPr sz="4400"><a:solidFill><a:schemeClr val="tx2"/></a:solidFill><a:latin typeface="+mj-lt"/></a:defRPr></a:lvl1pPr></p:titleStyle>
    <p:bodyStyle>
      <a:lvl1pPr marL="342900" indent="-342900"><a:buFont typeface="Arial"/><a:buChar char="&#8226;"/><a:defRPr sz="2800"/></a:lvl1pPr>
      <a:lvl2pPr marL="742950" indent="-285750"><a:buAutoNum type="arabicPeriod"/><a:defRPr sz="2400"/></a:lvl2pPr>
    </p:bodyStyle>
    <p:otherStyle><a:lvl1pPr><a:defRPr sz="1800"/></a:lvl1pPr></p:otherStyle>
  </p:txStyles>
</p:sldMaster>"#;

pub const LAYOUT: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
  xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
      <p:spPr><a:xfrm><a:off x="1000000" y="500000"/><a:ext cx="7000000" cy="1200000"/></a:xfrm></p:spPr>
      <p:txBody><a:bodyPr/><a:lstStyle><a:lvl1pPr algn="l"><a:defRPr sz="4000" i="1"/></a:lvl1pPr></a:lstStyle><a:p><a:endParaRPr/></a:p></p:txBody></p:sp>
    <p:sp><p:nvSpPr><p:cNvPr id="3" name="Content 2"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
      <p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p/></p:txBody></p:sp>
  </p:spTree></p:cSld>
  <p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
</p:sldLayout>"#;

pub const THEME: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office">
  <a:themeElements>
    <a:clrScheme name="Office">
      <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
      <a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
      <a:accent3><a:srgbClr val="A5A5A5"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4>
      <a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6>
      <a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
    </a:clrScheme>
    <a:fontScheme name="Office">
      <a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont>
      <a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont>
    </a:fontScheme>
    <a:fmtScheme name="Office">
      <a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"><a:tint val="40000"/></a:schemeClr></a:solidFill><a:gradFill><a:gsLst><a:gs pos="0"><a:schemeClr val="phClr"/></a:gs></a:gsLst></a:gradFill></a:fillStyleLst>
      <a:lnStyleLst><a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="19050"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst>
      <a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>
      <a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst>
    </a:fmtScheme>
  </a:themeElements>
</a:theme>"#;

/// 一个合法的 1x1 PNG。
pub const PNG_1X1: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\rIDATx\x9cc\xf8\xcf\xc0\xf0\x1f\0\x05\0\x01\xff\x89\x99=\x1d\0\0\0\0IEND\xaeB`\x82";

fn rels(items: &[(&str, &str, &str, bool)]) -> String {
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"{REL_NS}\">"
    );
    for (id, ty, target, external) in items {
        let mode = if *external {
            " TargetMode=\"External\""
        } else {
            ""
        };
        s.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{OD}/{ty}\" Target=\"{target}\"{mode}/>"
        ));
    }
    s.push_str("</Relationships>");
    s
}

/// 把任意字节当作 `ppt/slides/slide1.xml`,配上最小必需部件打成 `.pptx`。
///
/// slide rels 预置 `rId1` 版式、`rId2` 图片(`media/image1.png`)、`rId3` 外部超链接,
/// 让变异后的 XML 引用它们时能走到对应解析路径。
pub fn pack_slide_xml(slide_xml: &[u8]) -> Vec<u8> {
    let root = rels(&[("rId1", "officeDocument", "ppt/presentation.xml", false)]);
    let pres = rels(&[
        ("rId1", "slide", "slides/slide1.xml", false),
        (
            "rId2",
            "slideMaster",
            "slideMasters/slideMaster1.xml",
            false,
        ),
    ]);
    let slide = rels(&[
        (
            "rId1",
            "slideLayout",
            "../slideLayouts/slideLayout1.xml",
            false,
        ),
        ("rId2", "image", "../media/image1.png", false),
        ("rId3", "hyperlink", "https://example.com/", true),
    ]);
    let layout = rels(&[(
        "rId1",
        "slideMaster",
        "../slideMasters/slideMaster1.xml",
        false,
    )]);
    let master = rels(&[("rId1", "theme", "../theme/theme1.xml", false)]);
    pack(&[
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", root.as_bytes()),
        ("ppt/presentation.xml", PRESENTATION.as_bytes()),
        ("ppt/_rels/presentation.xml.rels", pres.as_bytes()),
        ("ppt/slides/slide1.xml", slide_xml),
        ("ppt/slides/_rels/slide1.xml.rels", slide.as_bytes()),
        ("ppt/slideLayouts/slideLayout1.xml", LAYOUT.as_bytes()),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            layout.as_bytes(),
        ),
        ("ppt/slideMasters/slideMaster1.xml", MASTER.as_bytes()),
        (
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            master.as_bytes(),
        ),
        ("ppt/theme/theme1.xml", THEME.as_bytes()),
        ("ppt/media/image1.png", PNG_1X1),
    ])
}

/// 按给定 `(部件名, 字节)` 打 zip(deflate)。
pub fn pack(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default();
        for (name, body) in parts {
            zip.start_file(*name, opts).expect("start_file");
            zip.write_all(body).expect("write");
        }
        zip.finish().expect("finish zip");
    }
    buf.into_inner()
}
