//! `ppt-parse` 验收测试:用 `zip` 写出一个最小但合法的 `.pptx`,断言 `parse_bytes`
//! 还原出文本框(段落 + 带样式 run)、表格(单元格 + 合并 + 填充)、画布尺寸与几何。
//!
//! 不落二进制 fixture —— pptx 在测试里现合成,确定性、自包含。

use std::io::{Cursor, Write};

use ppt_core::model::Shape;
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

/// 把上面的部件压成一个内存里的 `.pptx` zip 字节串。
fn build_minimal_pptx() -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default();
        for (name, body) in [
            ("[Content_Types].xml", CONTENT_TYPES),
            ("_rels/.rels", ROOT_RELS),
            ("ppt/presentation.xml", &presentation_xml()),
            ("ppt/_rels/presentation.xml.rels", PRESENTATION_RELS),
            ("ppt/slides/slide1.xml", SLIDE1),
            ("ppt/slides/_rels/slide1.xml.rels", SLIDE_RELS),
        ] {
            zip.start_file(name, opts).expect("start_file");
            zip.write_all(body.as_bytes()).expect("write");
        }
        zip.finish().expect("finish zip");
    }
    buf.into_inner()
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
    assert_eq!((rect.x, rect.y, rect.w, rect.h), (838200, 365125, 7772400, 1325563));
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
