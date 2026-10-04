//! 解析表格样式部件 `ppt/tableStyles.xml` -> `styleId -> TableStyle`。
//!
//! 每个 `a:tblStyle` 取九个部件(`wholeTbl` / `band1H` / `band2H` / `band1V` / `band2V` /
//! `firstRow` / `lastRow` / `firstCol` / `lastCol`):`a:tcStyle` 的纯色填充(`a:fill` 下
//! `solidFill` / `noFill`)与 `a:tcBdr` 六向边框(`a:ln`),`a:tcTxStyle` 的直接颜色与 `@b`。
//! 不支持的项整体跳过:角单元格(`nwCell` 等)、`tblBg`、渐变 / 图案填充、`fillRef` /
//! `lnRef` 主题引用、对角线、`cell3D`。结构深度固定,无递归下降;畸形输入尽力而为、绝不 panic。

use std::collections::BTreeMap;

use ppt_core::model::{TablePartStyle, TableStyle, TableStyleBorders};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::slide::{bare_ln, parse_ln_no_fill};
use super::text_style::{parse_color_in, parse_solid_fill};
use super::{attr_of, local_name, skip_element};

/// 解析一份 `tableStyles.xml`。同一 `styleId` 重复时首个获胜;缺 `styleId` 的样式丢弃。
pub fn parse(xml: &str) -> BTreeMap<String, TableStyle> {
    let mut out = BTreeMap::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"tblStyle" => {
                        let id = attr_of(&e, b"styleId").map(|s| s.trim().to_string());
                        let style = parse_tbl_style(&mut reader);
                        if let Some(id) = id.filter(|s| !s.is_empty()) {
                            out.entry(id).or_insert(style);
                        }
                    }
                    // tblStyleLst 根容器:继续下钻;其余整体跳过。
                    b"tblStyleLst" => {}
                    _ => skip_element(&mut reader, &name),
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// 解析 `a:tblStyle` 的各部件。已消费起始标签。
fn parse_tbl_style<R: std::io::BufRead>(reader: &mut Reader<R>) -> TableStyle {
    let mut style = TableStyle::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                let slot = match name.as_slice() {
                    b"wholeTbl" => Some(&mut style.whole_tbl),
                    b"band1H" => Some(&mut style.band1_h),
                    b"band2H" => Some(&mut style.band2_h),
                    b"band1V" => Some(&mut style.band1_v),
                    b"band2V" => Some(&mut style.band2_v),
                    b"firstRow" => Some(&mut style.first_row),
                    b"lastRow" => Some(&mut style.last_row),
                    b"firstCol" => Some(&mut style.first_col),
                    b"lastCol" => Some(&mut style.last_col),
                    // 角单元格 / tblBg / extLst 等不支持:跳过。
                    _ => None,
                };
                match slot {
                    Some(part) => *part = parse_part(reader),
                    None => skip_element(reader, &name),
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    style
}

/// 解析一个部件(`a:tcTxStyle` + `a:tcStyle`)。已消费部件起始标签。
fn parse_part<R: std::io::BufRead>(reader: &mut Reader<R>) -> TablePartStyle {
    let mut part = TablePartStyle::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"tcTxStyle" => {
                        part.bold = tx_bold(&e);
                        // 直接颜色子元素;`a:fontRef` / `a:font` 被 parse_color_in 跳过。
                        part.text_color = parse_color_in(reader);
                    }
                    b"tcStyle" => parse_tc_style(reader, &mut part),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"tcTxStyle" {
                    part.bold = tx_bold(&e);
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    part
}

/// `a:tcTxStyle@b`:`on` → 粗体,`off` → 非粗体,`def` / 缺失 → 未指定。
fn tx_bold(e: &BytesStart) -> Option<bool> {
    match attr_of(e, b"b").as_deref() {
        Some("on") => Some(true),
        Some("off") => Some(false),
        _ => None,
    }
}

/// 解析 `a:tcStyle`:`a:tcBdr` 边框 + `a:fill` 填充。已消费起始标签。
fn parse_tc_style<R: std::io::BufRead>(reader: &mut Reader<R>, part: &mut TablePartStyle) {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"tcBdr" => part.borders = parse_tc_bdr(reader),
                    b"fill" => part.fill = parse_fill(reader),
                    // fillRef / cell3D 不支持:跳过。
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
}

/// 解析 `a:tcStyle > a:fill`:`solidFill` → 纯色,`noFill` → 显式无填充;其它(渐变 /
/// 图案 / 图片)不支持 → 未指定。已消费起始标签。
fn parse_fill<R: std::io::BufRead>(
    reader: &mut Reader<R>,
) -> Option<Option<ppt_core::color::ColorSpec>> {
    let mut fill = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"solidFill" => fill = parse_solid_fill(reader).map(Some).or(fill),
                    b"noFill" => {
                        fill = Some(None);
                        skip_element(reader, &name);
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"noFill" {
                    fill = Some(None);
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    fill
}

/// 解析 `a:tcBdr` 的六向边框(对角线 `tl2br` / `tr2bl` 不支持,跳过)。已消费起始标签。
fn parse_tc_bdr<R: std::io::BufRead>(reader: &mut Reader<R>) -> TableStyleBorders {
    let mut b = TableStyleBorders::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                let slot = match name.as_slice() {
                    b"left" => Some(&mut b.left),
                    b"right" => Some(&mut b.right),
                    b"top" => Some(&mut b.top),
                    b"bottom" => Some(&mut b.bottom),
                    b"insideH" => Some(&mut b.inside_h),
                    b"insideV" => Some(&mut b.inside_v),
                    _ => None,
                };
                match slot {
                    Some(edge) => *edge = parse_edge(reader),
                    None => skip_element(reader, &name),
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    b
}

/// 解析一条边(`a:left` 等)内的 `a:ln`:含 `a:noFill` → 显式无线;有描边 → 画线;
/// 空 `a:ln` 或只有 `a:lnRef`(主题线条引用,不支持)→ 未指定。已消费起始标签。
fn parse_edge<R: std::io::BufRead>(
    reader: &mut Reader<R>,
) -> Option<Option<ppt_core::model::Stroke>> {
    let mut edge = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"ln" {
                    let (stroke, no_fill) = parse_ln_no_fill(reader, &e);
                    if no_fill {
                        edge = Some(None);
                    } else if stroke.is_some() {
                        edge = Some(stroke);
                    }
                } else {
                    skip_element(reader, &name);
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"ln" {
                    if let Some(s) = bare_ln(&e) {
                        edge = Some(Some(s));
                    }
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    edge
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppt_core::color::ColorSpec;

    const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

    #[test]
    fn parses_parts_fill_borders_and_text() {
        let xml = format!(
            r#"<a:tblStyleLst {NS} def="{{S}}">
              <a:tblStyle styleId="{{S}}" styleName="s">
                <a:wholeTbl>
                  <a:tcTxStyle b="off"><a:fontRef idx="minor"><a:schemeClr val="accent2"/></a:fontRef><a:schemeClr val="dk1"/></a:tcTxStyle>
                  <a:tcStyle>
                    <a:tcBdr>
                      <a:left><a:ln w="12700"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:ln></a:left>
                      <a:insideH><a:ln w="12700"><a:noFill/></a:ln></a:insideH>
                      <a:insideV><a:lnRef idx="1"><a:schemeClr val="accent1"/></a:lnRef></a:insideV>
                      <a:tl2br><a:ln w="12700"><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></a:ln></a:tl2br>
                    </a:tcBdr>
                    <a:fill><a:solidFill><a:schemeClr val="accent1"><a:tint val="40000"/></a:schemeClr></a:solidFill></a:fill>
                  </a:tcStyle>
                </a:wholeTbl>
                <a:band1H><a:tcStyle><a:fill><a:noFill/></a:fill></a:tcStyle></a:band1H>
                <a:firstRow><a:tcTxStyle b="on"/><a:tcStyle><a:fillRef idx="1"/></a:tcStyle></a:firstRow>
                <a:nwCell><a:tcStyle><a:fill><a:solidFill><a:srgbClr val="123456"/></a:solidFill></a:fill></a:tcStyle></a:nwCell>
              </a:tblStyle>
            </a:tblStyleLst>"#
        );
        let styles = parse(&xml);
        let s = styles.get("{S}").expect("style by id");
        let w = &s.whole_tbl;
        assert_eq!(w.bold, Some(false));
        assert_eq!(
            w.text_color,
            Some(ColorSpec::Scheme {
                name: "dk1".into(),
                transforms: vec![]
            }),
            "直接颜色子元素,不取 fontRef 内的"
        );
        assert!(matches!(&w.fill, Some(Some(ColorSpec::Scheme { name, .. })) if name == "accent1"));
        let left = w.borders.left.clone().flatten().expect("left line");
        assert_eq!(left.width_emu, Some(12_700));
        assert_eq!(w.borders.inside_h, Some(None), "noFill = 显式无线");
        assert_eq!(w.borders.inside_v, None, "lnRef 不支持 = 未指定");
        assert_eq!(s.band1_h.fill, Some(None), "fill > noFill");
        assert_eq!(s.first_row.bold, Some(true));
        assert_eq!(s.first_row.fill, None, "fillRef 不支持 = 未指定");
        assert_eq!(s.band2_h, TablePartStyle::default());
    }

    #[test]
    fn malformed_input_is_tolerated() {
        for xml in [
            "",
            "not xml at all <<<",
            r#"<a:tblStyleLst><a:tblStyle styleId="{T}"><a:wholeTbl><a:tcStyle><a:fill><a:solidFill"#,
            r#"<a:tblStyleLst><a:tblStyle><a:wholeTbl/></a:tblStyle></a:tblStyleLst>"#,
            r#"<a:tblStyle styleId="{U}"><a:wholeTbl><a:tcStyle></a:wholeTbl></a:tblStyle>"#,
        ] {
            let _ = parse(xml);
        }
        assert!(
            parse(r#"<a:tblStyleLst><a:tblStyle><a:wholeTbl/></a:tblStyle></a:tblStyleLst>"#)
                .is_empty(),
            "缺 styleId 的样式丢弃"
        );
    }

    #[test]
    fn duplicate_style_id_first_wins() {
        let xml = format!(
            r#"<a:tblStyleLst {NS}>
              <a:tblStyle styleId="{{D}}"><a:wholeTbl><a:tcTxStyle b="on"/></a:wholeTbl></a:tblStyle>
              <a:tblStyle styleId="{{D}}"><a:wholeTbl><a:tcTxStyle b="off"/></a:wholeTbl></a:tblStyle>
            </a:tblStyleLst>"#
        );
        assert_eq!(parse(&xml)["{D}"].whole_tbl.bold, Some(true));
    }
}
