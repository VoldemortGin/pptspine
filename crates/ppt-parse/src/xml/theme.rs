//! 解析主题部件 `ppt/theme/themeN.xml` -> [`Theme`](B-8):
//! `a:clrScheme`(12 色,`sysClr` 折 `lastClr`)、`a:fontScheme`(major/minor 的
//! latin/ea/cs)、`a:fmtScheme` 的 `fillStyleLst` / `lnStyleLst`(供 `p:style`
//! fillRef/lnRef 解析;非纯色项降级记 `None`)。

use ppt_core::theme::{ColorScheme, FontScheme, FontSet, Theme, ThemeLine};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::slide::parse_ln;
use super::text_style::parse_color_in;
use super::{attr_of, local_name, skip_element};

/// 解析一份主题 XML。容错:缺失部分取缺省(黑字白底)。
pub fn parse(xml: &str) -> Theme {
    let mut theme = Theme::default();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"clrScheme" => theme.color_scheme = parse_clr_scheme(&mut reader),
                    b"fontScheme" => theme.font_scheme = parse_font_scheme(&mut reader),
                    b"fillStyleLst" => theme.fill_styles = parse_fill_styles(&mut reader),
                    b"lnStyleLst" => theme.line_styles = parse_ln_styles(&mut reader),
                    // 这些容器可能藏同名元素(extraClrSchemeLst 内是完整 clrScheme;
                    // bgFillStyleLst 结构同 fillStyleLst)——整体跳过,不让其覆盖主值。
                    b"extraClrSchemeLst" | b"objectDefaults" | b"bgFillStyleLst" | b"extLst" => {
                        skip_element(&mut reader, &name)
                    }
                    // theme / themeElements / fmtScheme 等容器:继续下钻。
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    theme
}

/// 解析 `a:clrScheme` 的 12 个槽位。已消费起始标签。
fn parse_clr_scheme<R: std::io::BufRead>(reader: &mut Reader<R>) -> ColorScheme {
    let mut scheme = ColorScheme::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                // 槽位元素内是一个 srgbClr / sysClr;scheme 定义级不再有 scheme 引用。
                let color = parse_color_in(reader).and_then(|s| s.base_srgb());
                if let Some(c) = color {
                    match name.as_slice() {
                        b"dk1" => scheme.dk1 = c,
                        b"lt1" => scheme.lt1 = c,
                        b"dk2" => scheme.dk2 = c,
                        b"lt2" => scheme.lt2 = c,
                        b"accent1" => scheme.accent1 = c,
                        b"accent2" => scheme.accent2 = c,
                        b"accent3" => scheme.accent3 = c,
                        b"accent4" => scheme.accent4 = c,
                        b"accent5" => scheme.accent5 = c,
                        b"accent6" => scheme.accent6 = c,
                        b"hlink" => scheme.hlink = c,
                        b"folHlink" => scheme.fol_hlink = c,
                        _ => {}
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
    scheme
}

/// 解析 `a:fontScheme`(major/minor 的 latin/ea/cs `@typeface`)。已消费起始标签。
fn parse_font_scheme<R: std::io::BufRead>(reader: &mut Reader<R>) -> FontScheme {
    let mut fs = FontScheme::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"majorFont" => fs.major = parse_font_set(reader),
                    b"minorFont" => fs.minor = parse_font_set(reader),
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
    fs
}

/// 解析 `a:majorFont` / `a:minorFont` 的 latin/ea/cs。已消费起始标签。
fn parse_font_set<R: std::io::BufRead>(reader: &mut Reader<R>) -> FontSet {
    let mut set = FontSet::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                fill_font_slot(&mut set, &name, &e);
            }
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                fill_font_slot(&mut set, &name, &e);
                skip_element(reader, &name);
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    set
}

fn fill_font_slot(set: &mut FontSet, name: &[u8], e: &BytesStart) {
    let slot = match name {
        b"latin" => &mut set.latin,
        b"ea" => &mut set.ea,
        b"cs" => &mut set.cs,
        _ => return,
    };
    if slot.is_none() {
        *slot = attr_of(e, b"typeface").filter(|t| !t.is_empty());
    }
}

/// 解析 `a:fillStyleLst`:各项按序落位;纯色项记 spec,其它(渐变等)记 `None`。
/// 已消费起始标签。
fn parse_fill_styles<R: std::io::BufRead>(
    reader: &mut Reader<R>,
) -> Vec<Option<ppt_core::color::ColorSpec>> {
    let mut out = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"solidFill" {
                    out.push(parse_color_in(reader));
                } else {
                    out.push(None);
                    skip_element(reader, &name);
                }
            }
            Ok(Event::Empty(_)) => out.push(None),
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// 解析 `a:lnStyleLst`:各 `a:ln` 项 -> 宽度 + 颜色 spec。已消费起始标签。
fn parse_ln_styles<R: std::io::BufRead>(reader: &mut Reader<R>) -> Vec<ThemeLine> {
    let mut out = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"ln" {
                    let stroke = parse_ln(reader, &e);
                    out.push(ThemeLine {
                        color: stroke.as_ref().and_then(|s| s.color.clone()),
                        width_emu: stroke.as_ref().and_then(|s| s.width_emu),
                    });
                } else {
                    skip_element(reader, &name);
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"ln" {
                    out.push(ThemeLine {
                        color: None,
                        width_emu: attr_of(&e, b"w").and_then(|s| s.parse().ok()),
                    });
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}
