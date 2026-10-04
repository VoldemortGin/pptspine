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

// ---- 多部件 target(`parse_parts`)------------------------------------------------

/// 多部件 target 的部件种类数(`pack_part` 的 `selector` 对它取模)。
pub const PART_KINDS: u8 = 10;

const DGM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
const DSP: &str = "http://schemas.microsoft.com/office/drawing/2008/diagram";
const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const P_NS: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const C_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const TABLE_STYLE_ID: &str = "{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}";

/// 引用全部附属部件的最小 slide:文本(含 slidenum 字段)+ 套表格样式的表格 + 图表 + SmartArt。
fn multi_part_slide() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld {NS}><p:cSld><p:spTree>
<p:sp><p:nvSpPr><p:cNvPr id="2" name="Box"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
<p:spPr><a:xfrm><a:off x="838200" y="365125"/><a:ext cx="3000000" cy="500000"/></a:xfrm></p:spPr>
<p:txBody><a:bodyPr/><a:p><a:r><a:t>box</a:t></a:r><a:fld id="{{0}}" type="slidenum"><a:t>1</a:t></a:fld></a:p></p:txBody></p:sp>
<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="3" name="Table"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
<p:xfrm><a:off x="838200" y="1000000"/><a:ext cx="4000000" cy="800000"/></p:xfrm>
<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl>
<a:tblPr firstRow="1" bandRow="1"><a:tableStyleId>{TABLE_STYLE_ID}</a:tableStyleId></a:tblPr>
<a:tblGrid><a:gridCol w="2000000"/><a:gridCol w="2000000"/></a:tblGrid>
<a:tr h="400000"><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>h1</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>h2</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr>
<a:tr h="400000"><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>c1</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>c2</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr>
</a:tbl></a:graphicData></a:graphic></p:graphicFrame>
<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Chart"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
<p:xfrm><a:off x="838200" y="2000250"/><a:ext cx="4000000" cy="2500000"/></p:xfrm>
<a:graphic><a:graphicData uri="{C_NS}"><c:chart xmlns:c="{C_NS}" r:id="rId4"/></a:graphicData></a:graphic></p:graphicFrame>
<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="5" name="Diagram"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
<p:xfrm><a:off x="5000000" y="2000250"/><a:ext cx="3000000" cy="2500000"/></p:xfrm>
<a:graphic><a:graphicData uri="{DGM}"><dgm:relIds xmlns:dgm="{DGM}" r:dm="rId5" r:lo="rId9" r:qs="rId9" r:cs="rId9"/></a:graphicData></a:graphic></p:graphicFrame>
</p:spTree></p:cSld></p:sld>"#
    )
}

fn minimal_chart() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<c:chartSpace xmlns:c="{C_NS}" xmlns:a="{A_NS}"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>
<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>S!$B$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>Series</c:v></c:pt></c:strCache></c:strRef></c:tx>
<c:spPr><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></c:spPr>
<c:cat><c:strRef><c:f>S!$A$2:$A$3</c:f><c:strCache><c:ptCount val="2"/><c:pt idx="0"><c:v>A</c:v></c:pt><c:pt idx="1"><c:v>B</c:v></c:pt></c:strCache></c:strRef></c:cat>
<c:val><c:numRef><c:f>S!$B$2:$B$3</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
</c:barChart></c:plotArea></c:chart></c:chartSpace>"#
    )
}

fn minimal_table_styles() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:tblStyleLst xmlns:a="{A_NS}" def="{TABLE_STYLE_ID}"><a:tblStyle styleId="{TABLE_STYLE_ID}" styleName="Medium Style 2">
<a:wholeTbl><a:tcTxStyle><a:fontRef idx="minor"/><a:schemeClr val="dk1"/></a:tcTxStyle><a:tcStyle><a:tcBdr><a:left><a:ln w="12700"><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></a:ln></a:left></a:tcBdr><a:fill><a:solidFill><a:srgbClr val="CFD5EA"/></a:solidFill></a:fill></a:tcStyle></a:wholeTbl>
<a:firstRow><a:tcTxStyle b="on"><a:fontRef idx="minor"/><a:schemeClr val="lt1"/></a:tcTxStyle><a:tcStyle><a:fill><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></a:fill></a:tcStyle></a:firstRow>
</a:tblStyle></a:tblStyleLst>"#
    )
}

fn minimal_notes() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:notes xmlns:a="{A_NS}" xmlns:p="{P_NS}"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="2" name="Notes"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
<p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>speaker note</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#
    )
}

fn minimal_comments() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:cmLst xmlns:p="{P_NS}"><p:cm authorId="0" dt="2026-01-01T00:00:00.000" idx="1"><p:pos x="10" y="10"/><p:text>a comment</p:text></p:cm></p:cmLst>"#
    )
}

fn minimal_comment_authors() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:cmAuthorLst xmlns:p="{P_NS}"><p:cmAuthor id="0" name="Ann" initials="A" lastIdx="1" clrIdx="0"/></p:cmAuthorLst>"#
    )
}

fn minimal_diagram_data() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<dgm:dataModel xmlns:dgm="{DGM}" xmlns:a="{A_NS}"><dgm:ptLst>
<dgm:pt modelId="{{1}}" type="doc"><dgm:prSet/><dgm:spPr/><dgm:t><a:bodyPr/><a:p><a:endParaRPr lang="en-US"/></a:p></dgm:t></dgm:pt>
<dgm:pt modelId="{{2}}"><dgm:prSet/><dgm:spPr/><dgm:t><a:bodyPr/><a:p><a:r><a:t>node</a:t></a:r></a:p></dgm:t></dgm:pt>
</dgm:ptLst><dgm:cxnLst/><dgm:extLst><a:ext uri="http://schemas.microsoft.com/office/drawing/2008/diagram">
<dsp:dataModelExt xmlns:dsp="{DSP}" relId="rId8" minVer="{DGM}"/></a:ext></dgm:extLst></dgm:dataModel>"#
    )
}

fn minimal_diagram_drawing() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<dsp:drawing xmlns:dgm="{DGM}" xmlns:dsp="{DSP}" xmlns:a="{A_NS}"><dsp:spTree>
<dsp:nvGrpSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvGrpSpPr/></dsp:nvGrpSpPr><dsp:grpSpPr/>
<dsp:sp modelId="{{S}}"><dsp:nvSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvSpPr/></dsp:nvSpPr>
<dsp:spPr><a:xfrm><a:off x="100000" y="200000"/><a:ext cx="1000000" cy="500000"/></a:xfrm><a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></dsp:spPr>
<dsp:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>node</a:t></a:r></a:p></dsp:txBody></dsp:sp></dsp:spTree></dsp:drawing>"#
    )
}

/// 第 `kind` 种部件的最小合法内容(`kind` 含义见 [`pack_part`];也是 `make_seeds` 的种子)。
pub fn minimal_part(kind: u8) -> String {
    match kind % PART_KINDS {
        0 => LAYOUT.to_string(),
        1 => MASTER.to_string(),
        2 => THEME.to_string(),
        3 => minimal_chart(),
        4 => minimal_table_styles(),
        5 => minimal_notes(),
        6 => minimal_comments(),
        7 => minimal_diagram_drawing(),
        8 => minimal_diagram_data(),
        _ => PRESENTATION.to_string(),
    }
}

/// 按 `selector % PART_KINDS` 选一个部件(0 layout / 1 master / 2 theme / 3 chart / 4 tableStyles /
/// 5 notes / 6 comments / 7 diagram drawing / 8 diagram data / 9 presentation),用 `xml` 替换它;
/// 其余部件取最小合法内容,打成 `.pptx`。slide 引用全部附属部件(含批注作者、表格样式 id),
/// 让变异后的部件真能被解析到;slide 主部件另有 `parse_slide_xml` target 覆盖。
pub fn pack_part(selector: u8, xml: &[u8]) -> Vec<u8> {
    let kind = selector % PART_KINDS;
    let pick = |k: u8| -> Vec<u8> {
        if kind == k {
            xml.to_vec()
        } else {
            minimal_part(k).into_bytes()
        }
    };
    let (layout, master, theme, chart, table_styles) =
        (pick(0), pick(1), pick(2), pick(3), pick(4));
    let (notes, comments, drawing, data, presentation) =
        (pick(5), pick(6), pick(7), pick(8), pick(9));
    let slide = multi_part_slide();
    let root = rels(&[("rId1", "officeDocument", "ppt/presentation.xml", false)]);
    let pres_rels = rels(&[
        ("rId1", "slide", "slides/slide1.xml", false),
        (
            "rId2",
            "slideMaster",
            "slideMasters/slideMaster1.xml",
            false,
        ),
        ("rId3", "tableStyles", "tableStyles.xml", false),
        ("rId4", "commentAuthors", "commentAuthors.xml", false),
    ]);
    let slide_rels = rels(&[
        (
            "rId1",
            "slideLayout",
            "../slideLayouts/slideLayout1.xml",
            false,
        ),
        ("rId2", "image", "../media/image1.png", false),
        ("rId3", "hyperlink", "https://example.com/", true),
        ("rId4", "chart", "../charts/chart1.xml", false),
        ("rId5", "diagramData", "../diagrams/data1.xml", false),
        (
            "rId6",
            "notesSlide",
            "../notesSlides/notesSlide1.xml",
            false,
        ),
        ("rId7", "comments", "../comments/comment1.xml", false),
        ("rId8", "diagramDrawing", "../diagrams/drawing1.xml", false),
    ]);
    let layout_rels = rels(&[(
        "rId1",
        "slideMaster",
        "../slideMasters/slideMaster1.xml",
        false,
    )]);
    let master_rels = rels(&[("rId1", "theme", "../theme/theme1.xml", false)]);
    pack(&[
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", root.as_bytes()),
        ("ppt/presentation.xml", &presentation),
        ("ppt/_rels/presentation.xml.rels", pres_rels.as_bytes()),
        ("ppt/tableStyles.xml", &table_styles),
        (
            "ppt/commentAuthors.xml",
            minimal_comment_authors().as_bytes(),
        ),
        ("ppt/slides/slide1.xml", slide.as_bytes()),
        ("ppt/slides/_rels/slide1.xml.rels", slide_rels.as_bytes()),
        ("ppt/slideLayouts/slideLayout1.xml", &layout),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            layout_rels.as_bytes(),
        ),
        ("ppt/slideMasters/slideMaster1.xml", &master),
        (
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            master_rels.as_bytes(),
        ),
        ("ppt/theme/theme1.xml", &theme),
        ("ppt/charts/chart1.xml", &chart),
        ("ppt/diagrams/data1.xml", &data),
        ("ppt/diagrams/drawing1.xml", &drawing),
        ("ppt/notesSlides/notesSlide1.xml", &notes),
        ("ppt/comments/comment1.xml", &comments),
        ("ppt/media/image1.png", PNG_1X1),
    ])
}

/// 解析成功后跑 `to_text` / `to_markdown`(两种顺序 × 含隐藏页,带 / 不带终态 IR)与渲染映射
/// (`resolve`,PDF 渲染前的终态 IR),返回终态 IR 供调用方继续渲染。只要不 panic / 不 OOM 即可。
pub fn exercise_exports(parsed: &ppt_parse::ParsedPptx) -> ppt_core::ResolvedPresentation {
    use ppt_core::{presentation_markdown_with, presentation_text_with, ExportOptions, TextOrder};
    let resolved = ppt_parse::resolve(parsed);
    for order in [TextOrder::Visual, TextOrder::Document] {
        for include_hidden in [false, true] {
            let opts = ExportOptions {
                order,
                include_hidden,
                ..ExportOptions::default()
            };
            let _ = presentation_text_with(&parsed.presentation, None, &opts);
            let _ = presentation_text_with(&parsed.presentation, Some(&resolved), &opts);
            let _ = presentation_markdown_with(&parsed.presentation, None, &opts);
            let _ = presentation_markdown_with(&parsed.presentation, Some(&resolved), &opts);
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 十种部件各自被替换成畸形内容时,打出的包仍能解析(`Err` 也可,绝不 panic),导出器 / 渲染映射
    /// 也不 panic;用最小内容时各附属部件真被解析到(备注 / 批注 / 图表 / 表格样式 / SmartArt 进模型)。
    #[test]
    fn pack_part_covers_every_part_kind() {
        for selector in 0..PART_KINDS {
            for junk in [&b"<not-closed"[..], b"", b"\xff\xfe<a/>"] {
                if let Ok(parsed) = ppt_parse::parse_bytes(&pack_part(selector, junk)) {
                    exercise_exports(&parsed);
                }
            }
        }
        // 确定性"迷你 fuzz":每种部件的最小内容在若干截断点被砍断,走与 target 相同的链路
        // (解析 → 导出器 / 渲染映射 → 渲染 PDF)。不是真正的 fuzz,只是 `cargo test` 就能跑的冒烟。
        for selector in 0..PART_KINDS {
            let part = minimal_part(selector);
            let step = (part.len() / 8).max(1);
            for cut in (0..part.len()).step_by(step) {
                if let Ok(parsed) =
                    ppt_parse::parse_bytes(&pack_part(selector, &part.as_bytes()[..cut]))
                {
                    let resolved = exercise_exports(&parsed);
                    let _ = ppt_render::render_pdf(
                        &resolved,
                        &parsed.media,
                        &ppt_render::RenderOptions::default(),
                    );
                }
            }
        }
        // selector 取模:任何字节都落在合法种类。
        let _ = ppt_parse::parse_bytes(&pack_part(255, b""));

        let parsed = ppt_parse::parse_bytes(&pack_part(0, minimal_part(0).as_bytes()))
            .expect("minimal package parses");
        assert!(
            parsed.presentation.diagnostics.is_empty(),
            "{:?}",
            parsed.presentation.diagnostics
        );
        let slide = &parsed.presentation.slides[0];
        assert!(slide.notes.is_some(), "notes part not reached");
        assert!(!slide.comments.is_empty(), "comments part not reached");
        assert!(
            !parsed.inherit.table_styles.is_empty(),
            "tableStyles not reached"
        );
        assert!(!parsed.inherit.themes.is_empty() && !parsed.inherit.masters.is_empty());
        let dump = format!("{:?}", slide.shapes);
        assert!(dump.contains("Series"), "chart part not reached");
        assert!(
            dump.contains("roundRect"),
            "diagram drawing part not reached"
        );
        let resolved = exercise_exports(&parsed);
        assert_eq!(resolved.slides.len(), 1);
    }
}
