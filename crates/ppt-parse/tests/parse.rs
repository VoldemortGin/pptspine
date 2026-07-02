//! `ppt-parse` 验收测试:用 `zip` 写出一个最小但合法的 `.pptx`,断言 `parse_bytes`
//! 还原出文本框(段落 + 带样式 run)、表格(单元格 + 合并 + 填充)、画布尺寸与几何。
//!
//! 不落二进制 fixture —— pptx 在测试里现合成,确定性、自包含。

use std::io::{Cursor, Write};

use ppt_core::model::{RunKind, Shape, Stroke};
use ppt_parse::parse_bytes;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const SLIDE_CX: i64 = 9_144_000;
const SLIDE_CY: i64 = 6_858_000;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"#;

const PRESENTATION_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"#;

const SLIDE_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#;

const SLIDE1: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:spPr>
          <a:xfrm><a:off x="838200" y="365125"/><a:ext cx="7772400" cy="1325563"/></a:xfrm>
        </p:spPr>
        <p:txBody>
          <a:p>
            <a:pPr algn="ctr"/>
            <a:r>
              <a:rPr sz="4400" b="1">
                <a:solidFill><a:srgbClr val="1F4E79"/></a:solidFill>
                <a:latin typeface="Calibri"/>
              </a:rPr>
              <a:t>Hello pptspine</a:t>
            </a:r>
          </a:p>
        </p:txBody>
      </p:sp>
      <p:graphicFrame>
        <p:xfrm><a:off x="838200" y="2000250"/><a:ext cx="7772400" cy="2000250"/></p:xfrm>
        <a:graphic>
          <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
            <a:tbl>
              <a:tr h="370840">
                <a:tc>
                  <a:txBody><a:p><a:r><a:t>A1</a:t></a:r></a:p></a:txBody>
                  <a:tcPr><a:solidFill><a:srgbClr val="FFCC00"/></a:solidFill></a:tcPr>
                </a:tc>
                <a:tc gridSpan="1"><a:txBody><a:p><a:r><a:t>B1</a:t></a:r></a:p></a:txBody></a:tc>
              </a:tr>
              <a:tr h="370840">
                <a:tc><a:txBody><a:p><a:r><a:t>A2</a:t></a:r></a:p></a:txBody></a:tc>
                <a:tc><a:txBody><a:p><a:r><a:t>B2</a:t></a:r></a:p></a:txBody></a:tc>
              </a:tr>
            </a:tbl>
          </a:graphicData>
        </a:graphic>
      </p:graphicFrame>
    </p:spTree>
  </p:cSld>
</p:sld>"#;

/// 把部件压成一个内存里的 `.pptx` zip 字节串(slide1 内容可注入)。
fn build_pptx_with_slide(slide_xml: &str) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default();
        for (name, body) in [
            ("[Content_Types].xml", CONTENT_TYPES),
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", &presentation_xml()),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", slide_xml),
            ("ppt/slides/_rels/slide1.xml.rels", SLIDE_RELS),
        ] {
            zip.start_file(name, opts).expect("start_file");
            zip.write_all(body.as_bytes()).expect("write");
        }
        zip.finish().expect("finish zip");
    }
    buf.into_inner()
}

/// 默认的最小 `.pptx`(SLIDE1:文本框 + 2x2 表格)。
fn build_minimal_pptx() -> Vec<u8> {
    build_pptx_with_slide(SLIDE1)
}

/// 用给定的 `p:spTree` 内容合成一个单 slide 的 `.pptx`(B-3 止损测试共用)。
fn pptx_with_sp_tree(sp_tree_inner: &str) -> Vec<u8> {
    let slide = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>{sp_tree_inner}</p:spTree></p:cSld>
</p:sld>"#
    );
    build_pptx_with_slide(&slide)
}

/// 解析出唯一一张 slide 的形状列表。
fn shapes_of(sp_tree_inner: &str) -> Vec<Shape> {
    let pptx = pptx_with_sp_tree(sp_tree_inner);
    let parsed = parse_bytes(&pptx).expect("parse synthesized pptx");
    parsed.presentation.slides[0].shapes.clone()
}

fn presentation_xml() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
                xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
  <p:sldSz cx="{SLIDE_CX}" cy="{SLIDE_CY}" type="screen4x3"/>
</p:presentation>"#
    )
}

#[test]
fn parses_canvas_size_and_one_slide() {
    let pptx = build_minimal_pptx();
    let parsed = parse_bytes(&pptx).expect("parse minimal pptx");
    assert_eq!(parsed.presentation.slide_size, (SLIDE_CX, SLIDE_CY));
    assert_eq!(parsed.presentation.slides.len(), 1);
    assert_eq!(parsed.presentation.slides[0].index, 0);
}

#[test]
fn parses_textbox_runs_and_styling() {
    let pptx = build_minimal_pptx();
    let parsed = parse_bytes(&pptx).unwrap();
    let shapes = &parsed.presentation.slides[0].shapes;

    let tf = shapes
        .iter()
        .find_map(|s| match s {
            Shape::TextBox(tf) => Some(tf),
            _ => None,
        })
        .expect("a text box");

    assert_eq!(tf.paragraphs.len(), 1);
    let para = &tf.paragraphs[0];
    assert_eq!(para.align.as_deref(), Some("ctr"));
    let run = &para.runs[0];
    assert_eq!(run.text, "Hello pptspine");
    assert!(run.bold);
    assert_eq!(run.size_pt, Some(44.0));
    assert_eq!(run.font.as_deref(), Some("Calibri"));
    assert_eq!(run.color.map(|c| c.rgb), Some([0x1F, 0x4E, 0x79]));

    // 几何:EMU 矩形原样保留。
    let rect = tf.rect.expect("rect");
    assert_eq!(
        (rect.x, rect.y, rect.w, rect.h),
        (838200, 365125, 7772400, 1325563)
    );
}

#[test]
fn parses_table_cells_merges_and_fill() {
    let pptx = build_minimal_pptx();
    let parsed = parse_bytes(&pptx).unwrap();
    let shapes = &parsed.presentation.slides[0].shapes;

    let table = shapes
        .iter()
        .find_map(|s| match s {
            Shape::Table(t) => Some(t),
            _ => None,
        })
        .expect("a table");

    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.rows[0].cells.len(), 2);

    // A1:首格,带黄填充,跨度默认 1。
    let a1 = &table.rows[0].cells[0];
    let a1_text: String = a1.paragraphs[0]
        .runs
        .iter()
        .map(|r| r.text.as_str())
        .collect();
    assert_eq!(a1_text, "A1");
    assert_eq!(a1.col_span, 1);
    assert_eq!(a1.row_span, 1);
    assert!(!a1.merged);
    assert_eq!(a1.fill.map(|c| c.rgb), Some([0xFF, 0xCC, 0x00]));

    // B1 无填充。
    assert!(table.rows[0].cells[1].fill.is_none());

    // 表格几何。
    let rect = table.rect.expect("table rect");
    assert_eq!((rect.x, rect.y), (838200, 2000250));
}

#[test]
fn malformed_bytes_yield_error_not_panic() {
    // 非 zip 字节 -> Err(PptError),绝不 panic。
    let err = parse_bytes(b"not a pptx zip at all");
    assert!(err.is_err());
}

// ---- B-3 解析止损批(PRD-PDF-EXPORT §3.h/i/l/p/s/t/u)-----------------------

/// §3.i:`a:br` 段内硬换行不再丢——以 `"\n"` 文本的 Break run 落进段落。
#[test]
fn br_becomes_break_run() {
    let shapes = shapes_of(
        r#"<p:sp><p:txBody>
             <a:p>
               <a:r><a:t>Line one</a:t></a:r>
               <a:br/>
               <a:r><a:t>Line two</a:t></a:r>
             </a:p>
           </p:txBody></p:sp>"#,
    );
    let Shape::TextBox(tf) = &shapes[0] else {
        panic!("expected a text box");
    };
    let runs = &tf.paragraphs[0].runs;
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[0].kind, RunKind::Text);
    assert_eq!(runs[1].kind, RunKind::Break);
    assert_eq!(runs[1].text, "\n");
    assert_eq!(runs[2].text, "Line two");
    // 拼接文字自然还原两行。
    let joined: String = runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(joined, "Line one\nLine two");
}

/// §3.i:`a:br` 带 `a:rPr` 子元素(非自闭合)也一样保留换行。
#[test]
fn br_with_rpr_child_still_breaks() {
    let shapes = shapes_of(
        r#"<p:sp><p:txBody>
             <a:p>
               <a:r><a:t>a</a:t></a:r>
               <a:br><a:rPr sz="1800"/></a:br>
               <a:r><a:t>b</a:t></a:r>
             </a:p>
           </p:txBody></p:sp>"#,
    );
    let Shape::TextBox(tf) = &shapes[0] else {
        panic!("expected a text box");
    };
    let joined: String = tf.paragraphs[0]
        .runs
        .iter()
        .map(|r| r.text.as_str())
        .collect();
    assert_eq!(joined, "a\nb");
}

/// §3.i:`a:fld`(页码等字段)不再丢——缓存文本 + 字段类型 + 样式全保留。
#[test]
fn fld_becomes_field_run_with_cached_text() {
    let shapes = shapes_of(
        r#"<p:sp><p:txBody>
             <a:p>
               <a:fld id="{93A18523-9C96-4A83-A5F6-000000000000}" type="slidenum">
                 <a:rPr b="1"/>
                 <a:t>7</a:t>
               </a:fld>
             </a:p>
           </p:txBody></p:sp>"#,
    );
    let Shape::TextBox(tf) = &shapes[0] else {
        panic!("expected a text box");
    };
    let run = &tf.paragraphs[0].runs[0];
    assert_eq!(
        run.kind,
        RunKind::Field {
            field_type: Some("slidenum".to_string())
        }
    );
    assert_eq!(run.text, "7");
    assert!(run.bold);
}

/// §3.u:`mc:AlternateContent` 不再整块跳过——按锁定策略降入 `mc:Fallback`,
/// `mc:Choice`(可能带不认识的新命名空间)整体跳过。
#[test]
fn alternate_content_descends_into_fallback() {
    let shapes = shapes_of(
        r#"<mc:AlternateContent>
             <mc:Choice Requires="cx1">
               <p:sp><p:txBody><a:p><a:r><a:t>CHOICE</a:t></a:r></a:p></p:txBody></p:sp>
             </mc:Choice>
             <mc:Fallback>
               <p:sp><p:txBody><a:p><a:r><a:t>FALLBACK</a:t></a:r></a:p></p:txBody></p:sp>
             </mc:Fallback>
           </mc:AlternateContent>"#,
    );
    assert_eq!(shapes.len(), 1, "exactly the Fallback shape");
    let Shape::TextBox(tf) = &shapes[0] else {
        panic!("expected a text box from mc:Fallback");
    };
    assert_eq!(tf.paragraphs[0].runs[0].text, "FALLBACK");
}

/// §3.t:`p:cxnSp` 连接线不再被丢——几何名 / 矩形 / 描边(色 + 宽 + 虚线)全保留。
#[test]
fn cxn_sp_parses_as_connector() {
    let shapes = shapes_of(
        r#"<p:cxnSp>
             <p:nvCxnSpPr><p:cNvPr id="4" name="Straight Connector 3"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr>
             <p:spPr>
               <a:xfrm><a:off x="100" y="200"/><a:ext cx="300" cy="400"/></a:xfrm>
               <a:prstGeom prst="straightConnector1"><a:avLst/></a:prstGeom>
               <a:ln w="19050">
                 <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
                 <a:prstDash val="dash"/>
               </a:ln>
             </p:spPr>
           </p:cxnSp>"#,
    );
    let Shape::Connector(c) = &shapes[0] else {
        panic!("expected a connector");
    };
    let rect = c.rect.expect("connector rect");
    assert_eq!((rect.x, rect.y, rect.w, rect.h), (100, 200, 300, 400));
    assert_eq!(c.geometry.as_deref(), Some("straightConnector1"));
    let stroke = c.stroke.as_ref().expect("connector stroke");
    assert_eq!(stroke.color.map(|c| c.rgb), Some([0xFF, 0x00, 0x00]));
    assert_eq!(stroke.width_emu, Some(19050));
    assert_eq!(stroke.dash.as_deref(), Some("dash"));
}

/// §3.s:非表格 `p:graphicFrame`(图表 / SmartArt / OLE)不再连矩形一起消失——
/// 降级为占位形状,保留外框与 `graphicData@uri`。
#[test]
fn non_table_graphic_frame_keeps_rect_as_placeholder() {
    let shapes = shapes_of(
        r#"<p:graphicFrame>
             <p:xfrm><a:off x="1000" y="2000"/><a:ext cx="3000" cy="4000"/></p:xfrm>
             <a:graphic>
               <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">
                 <c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId9"/>
               </a:graphicData>
             </a:graphic>
           </p:graphicFrame>"#,
    );
    let Shape::Placeholder(ph) = &shapes[0] else {
        panic!("expected a graphic placeholder");
    };
    let rect = ph.rect.expect("placeholder rect");
    assert_eq!((rect.x, rect.y, rect.w, rect.h), (1000, 2000, 3000, 4000));
    assert_eq!(
        ph.kind.as_deref(),
        Some("http://schemas.openxmlformats.org/drawingml/2006/chart")
    );
}

/// §3.h:`a:ea`/`a:cs` 字体(CJK 关键)与 `@u`/`@strike` 属性落进 run;
/// 显式关闭值(`u="none"` / `strike="noStrike"`)不算开启。
#[test]
fn ea_cs_fonts_and_underline_strike() {
    let shapes = shapes_of(
        r#"<p:sp><p:txBody>
             <a:p>
               <a:r>
                 <a:rPr sz="1800" u="sng" strike="sngStrike">
                   <a:latin typeface="Calibri"/>
                   <a:ea typeface="SimSun"/>
                   <a:cs typeface="Arial"/>
                 </a:rPr>
                 <a:t>styled</a:t>
               </a:r>
               <a:r><a:rPr u="none" strike="noStrike"/><a:t>plain</a:t></a:r>
             </a:p>
           </p:txBody></p:sp>"#,
    );
    let Shape::TextBox(tf) = &shapes[0] else {
        panic!("expected a text box");
    };
    let styled = &tf.paragraphs[0].runs[0];
    assert!(styled.underline);
    assert!(styled.strike);
    assert_eq!(styled.font.as_deref(), Some("Calibri"));
    assert_eq!(styled.ea_font.as_deref(), Some("SimSun"));
    assert_eq!(styled.cs_font.as_deref(), Some("Arial"));

    let plain = &tf.paragraphs[0].runs[1];
    assert!(!plain.underline);
    assert!(!plain.strike);
    assert_eq!(plain.ea_font, None);
}

/// §3.l:`a:ln` 线宽 / 虚线预设落进 `Stroke`;空 `a:ln` 仍不产生描边
/// (保持旧的文本框 / 自选图形分类行为)。
#[test]
fn ln_width_and_dash_on_autoshape() {
    let shapes = shapes_of(
        r#"<p:sp>
             <p:spPr>
               <a:prstGeom prst="rect"/>
               <a:ln w="25400">
                 <a:solidFill><a:srgbClr val="00FF00"/></a:solidFill>
                 <a:prstDash val="sysDot"/>
               </a:ln>
             </p:spPr>
           </p:sp>
           <p:sp>
             <p:spPr><a:ln/></p:spPr>
             <p:txBody><a:p><a:r><a:t>still a text box</a:t></a:r></a:p></p:txBody>
           </p:sp>"#,
    );
    let Shape::Auto(auto) = &shapes[0] else {
        panic!("expected an autoshape");
    };
    assert_eq!(
        auto.stroke,
        Some(Stroke {
            color: ppt_core::model::Color::from_hex("00FF00"),
            width_emu: Some(25400),
            dash: Some("sysDot".to_string()),
        })
    );
    // 空 `<a:ln/>` 不构成描边 -> 第二个 sp 仍是纯文本框。
    assert!(matches!(&shapes[1], Shape::TextBox(_)));
}

/// §3.s 附带回归:`p:graphicFrame` 之后的同级形状不再被静默吞掉
/// (穿透 `a:graphic`/`a:graphicData` 时按深度计数,不再见 End 就 break)。
#[test]
fn shapes_after_graphic_frame_survive() {
    let shapes = shapes_of(
        r#"<p:graphicFrame>
             <p:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></p:xfrm>
             <a:graphic>
               <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
                 <a:tbl>
                   <a:tr h="1"><a:tc><a:txBody><a:p><a:r><a:t>cell</a:t></a:r></a:p></a:txBody></a:tc></a:tr>
                 </a:tbl>
               </a:graphicData>
             </a:graphic>
           </p:graphicFrame>
           <p:sp><p:txBody><a:p><a:r><a:t>after frame</a:t></a:r></a:p></p:txBody></p:sp>"#,
    );
    assert_eq!(shapes.len(), 2, "the shape after the frame must survive");
    assert!(matches!(&shapes[0], Shape::Table(_)));
    let Shape::TextBox(tf) = &shapes[1] else {
        panic!("expected the trailing text box");
    };
    assert_eq!(tf.paragraphs[0].runs[0].text, "after frame");
}

/// §3.p:`a:tblGrid` 列宽落进 `Table.col_widths`(自闭合与带 extLst 子元素两种形式)。
#[test]
fn tbl_grid_col_widths_parsed() {
    let shapes = shapes_of(
        r#"<p:graphicFrame>
             <p:xfrm><a:off x="0" y="0"/><a:ext cx="5120767" cy="741680"/></p:xfrm>
             <a:graphic>
               <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
                 <a:tbl>
                   <a:tblGrid>
                     <a:gridCol w="3886200"/>
                     <a:gridCol w="1234567"><a:extLst/></a:gridCol>
                   </a:tblGrid>
                   <a:tr h="370840">
                     <a:tc><a:txBody><a:p><a:r><a:t>A1</a:t></a:r></a:p></a:txBody></a:tc>
                     <a:tc><a:txBody><a:p><a:r><a:t>B1</a:t></a:r></a:p></a:txBody></a:tc>
                   </a:tr>
                 </a:tbl>
               </a:graphicData>
             </a:graphic>
           </p:graphicFrame>"#,
    );
    let Shape::Table(t) = &shapes[0] else {
        panic!("expected a table");
    };
    assert_eq!(t.col_widths, vec![3886200, 1234567]);
    assert_eq!(t.rows.len(), 1);
    // 既有字段不受影响。
    assert_eq!(t.rows[0].cells.len(), 2);
}
