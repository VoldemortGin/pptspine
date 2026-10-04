//! 生成种子语料(现场构造,不落二进制 fixture;`fuzz/corpus/` 已 .gitignore)。
//!
//! 用法(从仓库根):`cargo run --manifest-path fuzz/Cargo.toml --bin make_seeds`
//! 写入 `fuzz/corpus/{parse_pptx,parse_slide_xml,parse_parts,render_pdf}/`:
//! - `parse_slide_xml`:裸 `slide1.xml`;
//! - `parse_pptx`:完整 pptx(含 layout / master / theme / 图片 / 超链接 rels);
//! - `parse_parts`:`[种类字节] + 该种部件的最小合法 XML`(每种一个);
//! - `render_pdf`:同 `parse_pptx` 的 pptx 种子 + 裸 `slide1.xml`(见 target 说明)。

use std::fs;
use std::path::Path;

use pptspine_fuzz::{minimal_part, pack_slide_xml, NS, PART_KINDS};

/// `p:spTree` 内容片段,每个对应一类解析路径。
const TREES: &[(&str, &str)] = &[
    (
        "plain_text",
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Box"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
<p:spPr><a:xfrm><a:off x="838200" y="365125"/><a:ext cx="7772400" cy="1325563"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
<p:txBody><a:bodyPr wrap="square"/><a:lstStyle/>
<a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="en-US" sz="4400" b="1" i="1" u="sng"><a:solidFill><a:srgbClr val="1F4E79"/></a:solidFill><a:latin typeface="Calibri"/></a:rPr><a:t>Hello pptspine</a:t></a:r></a:p>
<a:p><a:r><a:rPr sz="2000" baseline="30000"/><a:t>second line</a:t></a:r><a:br/><a:r><a:t>third</a:t></a:r></a:p></p:txBody></p:sp>"#,
    ),
    (
        "inherit_list",
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="ctrTitle"/></p:nvPr></p:nvSpPr><p:spPr/>
<p:txBody><a:bodyPr/><a:p><a:r><a:t>Deck Title</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="3" name="Content 2"/><p:cNvSpPr/><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:spPr/>
<p:txBody><a:bodyPr><a:normAutofit fontScale="90000"/></a:bodyPr>
<a:p><a:r><a:t>first level</a:t></a:r></a:p>
<a:p><a:pPr lvl="1"/><a:r><a:t>second level</a:t></a:r></a:p>
<a:p><a:pPr marL="457200" indent="-457200"><a:buAutoNum type="romanLcPeriod" startAt="3"/></a:pPr><a:r><a:t>numbered</a:t></a:r></a:p></p:txBody></p:sp>"#,
    ),
    (
        "table",
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Table"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
<p:xfrm><a:off x="838200" y="2000250"/><a:ext cx="7772400" cy="2000250"/></p:xfrm>
<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl>
<a:tblPr firstRow="1" bandRow="1"/><a:tblGrid><a:gridCol w="3886200"/><a:gridCol w="3886200"/></a:tblGrid>
<a:tr h="370840"><a:tc gridSpan="2"><a:txBody><a:bodyPr/><a:p><a:r><a:t>merged</a:t></a:r></a:p></a:txBody><a:tcPr><a:solidFill><a:srgbClr val="FFCC00"/></a:solidFill></a:tcPr></a:tc><a:tc hMerge="1"><a:txBody><a:bodyPr/><a:p/></a:txBody></a:tc></a:tr>
<a:tr h="370840"><a:tc rowSpan="2"><a:txBody><a:bodyPr/><a:p><a:r><a:t>A2</a:t></a:r></a:p></a:txBody><a:tcPr marL="91440"><a:lnL w="12700"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:lnL></a:tcPr></a:tc><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>B2</a:t></a:r></a:p></a:txBody></a:tc></a:tr>
<a:tr h="370840"><a:tc vMerge="1"><a:txBody><a:bodyPr/><a:p/></a:txBody></a:tc><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>B3</a:t></a:r></a:p></a:txBody></a:tc></a:tr>
</a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#,
    ),
    (
        "picture_connector",
        r#"<p:pic><p:nvPicPr><p:cNvPr id="5" name="Pic" descr="alt text"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
<p:blipFill><a:blip r:embed="rId2"/><a:srcRect l="1000" t="1000"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>
<p:spPr><a:xfrm rot="900000" flipH="1"><a:off x="914400" y="914400"/><a:ext cx="1828800" cy="1828800"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>
<p:cxnSp><p:nvCxnSpPr><p:cNvPr id="6" name="Conn"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr>
<p:spPr><a:xfrm flipV="1"><a:off x="3000000" y="1000000"/><a:ext cx="3000000" cy="2000000"/></a:xfrm><a:prstGeom prst="bentConnector3"><a:avLst><a:gd name="adj1" fmla="val 50000"/></a:avLst></a:prstGeom>
<a:ln w="28575" cap="rnd"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:prstDash val="dash"/><a:headEnd type="oval" w="lg" len="lg"/><a:tailEnd type="triangle"/></a:ln></p:spPr></p:cxnSp>"#,
    ),
    (
        "group_custgeom",
        r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="7" name="Group"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
<p:grpSpPr><a:xfrm rot="1800000"><a:off x="1000000" y="1000000"/><a:ext cx="4000000" cy="3000000"/><a:chOff x="0" y="0"/><a:chExt cx="2000" cy="1500"/></a:xfrm></p:grpSpPr>
<p:sp><p:nvSpPr><p:cNvPr id="8" name="Free"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
<p:spPr><a:xfrm><a:off x="100" y="100"/><a:ext cx="1000" cy="800"/></a:xfrm>
<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l="0" t="0" r="r" b="b"/><a:pathLst><a:path w="1000" h="800"><a:moveTo><a:pt x="0" y="0"/></a:moveTo><a:lnTo><a:pt x="1000" y="0"/></a:lnTo><a:cubicBezTo><a:pt x="1000" y="400"/><a:pt x="500" y="800"/><a:pt x="0" y="800"/></a:cubicBezTo><a:arcTo wR="100" hR="100" stAng="0" swAng="5400000"/><a:close/></a:path></a:pathLst></a:custGeom>
<a:gradFill><a:gsLst><a:gs pos="0"><a:schemeClr val="accent1"/></a:gs><a:gs pos="100000"><a:srgbClr val="FFFFFF"/></a:gs></a:gsLst><a:lin ang="5400000"/></a:gradFill></p:spPr></p:sp>
<p:grpSp><p:nvGrpSpPr><p:cNvPr id="9" name="Inner"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="500" cy="500"/><a:chOff x="0" y="0"/><a:chExt cx="500" cy="500"/></a:xfrm></p:grpSpPr>
<p:sp><p:nvSpPr><p:cNvPr id="10" name="Ellipse"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom></p:spPr><p:style><a:lnRef idx="2"><a:schemeClr val="accent1"/></a:lnRef><a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef><a:effectRef idx="0"><a:schemeClr val="accent1"/></a:effectRef><a:fontRef idx="minor"/></p:style></p:sp></p:grpSp>
</p:grpSp>"#,
    ),
    (
        "alt_content_empty_group_choice",
        r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main">
<mc:Choice Requires="p14"><p:grpSp><p:nvGrpSpPr><p:cNvPr id="2" name="ink"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p14:contentPart r:id="rId5"/></p:grpSp></mc:Choice>
<mc:Fallback><p:sp><p:nvSpPr><p:cNvPr id="3" name="t"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>FALLBACK TEXT</a:t></a:r></a:p></p:txBody></p:sp></mc:Fallback>
</mc:AlternateContent>"#,
    ),
    (
        "smartart_many_frames",
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="13" name="D"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:relIds xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" r:dm="rId9"/></a:graphicData></a:graphic></p:graphicFrame>
<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="14" name="D2"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:relIds xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" r:dm="rId9"/></a:graphicData></a:graphic></p:graphicFrame>"#,
    ),
    (
        "link_chart_placeholder",
        r#"<p:sp><p:nvSpPr><p:cNvPr id="11" name="Link"><a:hlinkClick r:id="rId3" tooltip="tip"/></p:cNvPr><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>
<p:spPr><a:xfrm><a:off x="838200" y="365125"/><a:ext cx="3000000" cy="500000"/></a:xfrm></p:spPr>
<p:txBody><a:bodyPr/><a:p><a:r><a:rPr><a:hlinkClick r:id="rId3"/></a:rPr><a:t>link text</a:t></a:r><a:fld id="{00000000-0000-0000-0000-000000000000}" type="slidenum"><a:t>1</a:t></a:fld></a:p></p:txBody></p:sp>
<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="12" name="Chart"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
<p:xfrm><a:off x="838200" y="2000250"/><a:ext cx="5000000" cy="3000000"/></p:xfrm>
<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId9"/></a:graphicData></a:graphic></p:graphicFrame>"#,
    ),
];

/// "短 XML → 大模型"的放大种子(形状 / 段落 / run / 表格单元格洪水,规模取小以便变异):
/// 让模糊测试从一开始就覆盖解析时的形状 / 节点预算分支。
fn flood_trees() -> Vec<(&'static str, String)> {
    let text_box = |inner: &str| {
        format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/>{inner}</p:txBody></p:sp>"#
        )
    };
    vec![
        ("flood_shapes", "<p:cxnSp></p:cxnSp>".repeat(256)),
        ("flood_paragraphs", text_box(&"<a:p/>".repeat(256))),
        (
            "flood_runs",
            text_box(&format!(
                "<a:p>{}</a:p>",
                "<a:br/><a:r><a:t>x</a:t></a:r>".repeat(256)
            )),
        ),
        (
            "flood_table_cells",
            format!(
                r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="T"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblGrid>{}</a:tblGrid>{}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#,
                r#"<a:gridCol w="1"/>"#.repeat(64),
                format!("<a:tr h=\"1\">{}</a:tr>", "<a:tc/>".repeat(64)).repeat(16)
            ),
        ),
    ]
}

fn slide_xml(name: &str, tree: &str) -> String {
    // 最后一类种子同时带 `show="0"`(隐藏页)。
    let show = if name == "link_chart_placeholder" {
        " show=\"0\""
    } else {
        ""
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sld {NS}{show}>\n  <p:cSld><p:spTree>{tree}</p:spTree></p:cSld>\n</p:sld>"
    )
}

fn write(dir: &Path, name: &str, bytes: &[u8]) {
    fs::create_dir_all(dir).expect("mkdir corpus dir");
    fs::write(dir.join(name), bytes).expect("write seed");
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    let mut count = 0;
    let trees = TREES
        .iter()
        .map(|(n, t)| (*n, (*t).to_string()))
        .chain(flood_trees());
    for (name, tree) in trees {
        let xml = slide_xml(name, &tree);
        let pptx = pack_slide_xml(xml.as_bytes());
        write(
            &root.join("parse_slide_xml"),
            &format!("{name}.xml"),
            xml.as_bytes(),
        );
        write(&root.join("parse_pptx"), &format!("{name}.pptx"), &pptx);
        write(&root.join("render_pdf"), &format!("{name}.pptx"), &pptx);
        write(
            &root.join("render_pdf"),
            &format!("{name}.xml"),
            xml.as_bytes(),
        );
        count += 1;
    }
    for kind in 0..PART_KINDS {
        let mut seed = vec![kind];
        seed.extend_from_slice(minimal_part(kind).as_bytes());
        write(
            &root.join("parse_parts"),
            &format!("part_{kind}.bin"),
            &seed,
        );
    }
    println!("wrote seeds for {count} decks under {}", root.display());
}
