//! 继承链共享的样式 walker(slide / layout / master / presentation 复用):
//! - 颜色 spec:`a:srgbClr` / `a:sysClr` / `a:schemeClr` + 修饰变换(lumMod 等);
//! - run 样式:`a:rPr` / `a:defRPr` 形(属性三态 + latin/ea/cs + solidFill);
//! - 层级段落样式:`a:pPr` / `a:lvlNpPr` 形(algn/marL/indent + bu* + defRPr);
//! - 列表样式:`a:lstStyle` / master `p:txStyles` 桶(lvl1pPr..lvl9pPr)。
//!
//! 容错同家族约定:未知元素跳过、缺失属性 → `None`、绝不 panic。

use ppt_core::color::{ColorSpec, ColorTransform};
use ppt_core::model::Color;
use ppt_core::style::{Bullet, RunStyle, TextLevelStyle, TextStyleLevels};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::{attr_of, local_name, ooxml_bool, skip_element};

// ---- 颜色 spec ------------------------------------------------------------

/// 从一个颜色元素起始标签建**基础** spec(变换后填)。非颜色元素返回 `None`。
/// `a:sysClr` 折算为其 `lastClr` 缓存值(缺失时按 `val` 给黑/白兜底)。
fn base_spec(name: &[u8], e: &BytesStart) -> Option<ColorSpec> {
    match name {
        b"srgbClr" => attr_of(e, b"val")
            .and_then(|h| Color::from_hex(&h))
            .map(|c| ColorSpec::srgb(c.rgb)),
        b"sysClr" => {
            let rgb = attr_of(e, b"lastClr")
                .and_then(|h| Color::from_hex(&h))
                .map(|c| c.rgb)
                .unwrap_or_else(|| match attr_of(e, b"val").as_deref() {
                    Some("window") => [0xFF, 0xFF, 0xFF],
                    _ => [0, 0, 0],
                });
            Some(ColorSpec::Srgb {
                rgb,
                transforms: Vec::new(),
            })
        }
        b"schemeClr" => attr_of(e, b"val").map(|name| ColorSpec::Scheme {
            name,
            transforms: Vec::new(),
        }),
        _ => None,
    }
}

/// 解析一个颜色元素的修饰变换子元素,直到其结束标签。已消费该颜色元素起始标签。
fn parse_transforms<R: std::io::BufRead>(reader: &mut Reader<R>) -> Vec<ColorTransform> {
    let mut out = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) => {
                if let Some(t) = transform_of(local_name(e.name().as_ref()), &e) {
                    out.push(t);
                }
            }
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if let Some(t) = transform_of(&name, &e) {
                    out.push(t);
                }
                skip_element(reader, &name);
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

/// 识别一个变换元素(`val` 为千分之一个百分点定点数)。未知名 → `None`(跳过)。
fn transform_of(name: &[u8], e: &BytesStart) -> Option<ColorTransform> {
    let val: i64 = attr_of(e, b"val")?.parse().ok()?;
    Some(match name {
        b"lumMod" => ColorTransform::LumMod(val),
        b"lumOff" => ColorTransform::LumOff(val),
        b"tint" => ColorTransform::Tint(val),
        b"shade" => ColorTransform::Shade(val),
        b"alpha" => ColorTransform::Alpha(val),
        b"satMod" => ColorTransform::SatMod(val),
        _ => return None,
    })
}

/// 把 `transforms` 装回 spec。
fn with_transforms(spec: ColorSpec, transforms: Vec<ColorTransform>) -> ColorSpec {
    match spec {
        ColorSpec::Srgb { rgb, .. } => ColorSpec::Srgb { rgb, transforms },
        ColorSpec::Scheme { name, .. } => ColorSpec::Scheme { name, transforms },
    }
}

/// 在一个已打开的容器元素(如 `a:solidFill`、`a:fillRef`)内解析第一个颜色元素
/// (含变换),消费到容器结束标签。已消费容器起始标签。
pub fn parse_color_in<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<ColorSpec> {
    let mut spec = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if spec.is_none() {
                    spec = base_spec(&name, &e);
                }
            }
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match base_spec(&name, &e) {
                    Some(base) if spec.is_none() => {
                        spec = Some(with_transforms(base, parse_transforms(reader)));
                    }
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
    spec
}

/// 解析 `a:solidFill` -> 颜色 spec。已消费 `<a:solidFill>` 起始标签。
pub fn parse_solid_fill<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<ColorSpec> {
    parse_color_in(reader)
}

// ---- run 样式 --------------------------------------------------------------

/// `a:rPr` / `a:defRPr` 属性上的 run 样式(三态:属性缺失 → `None`)。
pub fn run_style_attrs(e: &BytesStart) -> RunStyle {
    RunStyle {
        size_pt: attr_of(e, b"sz")
            .and_then(|s| s.parse::<f32>().ok())
            .map(|v| v / 100.0),
        bold: attr_of(e, b"b").map(ooxml_bool),
        italic: attr_of(e, b"i").map(ooxml_bool),
        // `u="none"` / `strike="noStrike"` 是显式关闭(`Some(false)`)。
        underline: attr_of(e, b"u").map(|v| v != "none"),
        strike: attr_of(e, b"strike").map(|v| v != "noStrike"),
        ..RunStyle::default()
    }
}

/// 解析一个 run 样式元素(`a:rPr` / `a:defRPr`,非自闭合形式):属性 + 子元素
/// `a:latin`/`a:ea`/`a:cs`(@typeface)与 `a:solidFill`(颜色)。已消费起始标签。
pub fn parse_run_style<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    start: &BytesStart,
) -> RunStyle {
    let mut rs = run_style_attrs(start);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"latin" | b"ea" | b"cs" => {
                        set_typeface(&mut rs, &name, &e);
                        skip_element(reader, &name);
                    }
                    b"solidFill" => {
                        let c = parse_solid_fill(reader);
                        rs.color = c.or(rs.color);
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if matches!(name.as_slice(), b"latin" | b"ea" | b"cs") {
                    set_typeface(&mut rs, &name, &e);
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    rs
}

/// 把 `a:latin`/`a:ea`/`a:cs` 的 `@typeface` 填进对应槽位(已有值不覆盖;空串按缺省)。
fn set_typeface(rs: &mut RunStyle, name: &[u8], e: &BytesStart) {
    let slot = match name {
        b"latin" => &mut rs.font,
        b"ea" => &mut rs.ea_font,
        _ => &mut rs.cs_font,
    };
    if slot.is_none() {
        *slot = attr_of(e, b"typeface").filter(|t| !t.is_empty());
    }
}

// ---- 层级段落样式 ----------------------------------------------------------

/// `a:pPr` / `a:lvlNpPr` 属性上的段落样式(algn / marL / indent)。
pub fn level_style_attrs(e: &BytesStart) -> TextLevelStyle {
    TextLevelStyle {
        align: attr_of(e, b"algn"),
        mar_l: attr_of(e, b"marL").and_then(|s| s.parse().ok()),
        indent: attr_of(e, b"indent").and_then(|s| s.parse().ok()),
        ..TextLevelStyle::default()
    }
}

/// 解析一个段落样式元素(`a:pPr` / `a:lvlNpPr` / `a:defPPr`,非自闭合形式):
/// 属性 + 子元素 `a:buNone`/`a:buChar`/`a:buAutoNum`/`a:buFont`/`a:buSzPct`/`a:defRPr`。
/// 已消费起始标签。
pub fn parse_level_style<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    start: &BytesStart,
) -> TextLevelStyle {
    let mut ls = level_style_attrs(start);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                fill_level_child(&mut ls, &name, &e);
                if name.as_slice() == b"defRPr" {
                    ls.def_rpr = Some(run_style_attrs(&e));
                }
            }
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"defRPr" {
                    ls.def_rpr = Some(parse_run_style(reader, &e));
                } else {
                    fill_level_child(&mut ls, &name, &e);
                    skip_element(reader, &name);
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    ls
}

/// 识别 bu* 子元素并填入(`defRPr` 由调用方处理)。
fn fill_level_child(ls: &mut TextLevelStyle, name: &[u8], e: &BytesStart) {
    match name {
        b"buNone" => ls.bullet = Some(Bullet::None),
        b"buChar" => {
            if let Some(ch) = attr_of(e, b"char") {
                ls.bullet = Some(Bullet::Char(ch));
            }
        }
        b"buAutoNum" => {
            ls.bullet = Some(Bullet::AutoNum {
                scheme: attr_of(e, b"type"),
                start_at: attr_of(e, b"startAt").and_then(|s| s.parse().ok()),
            });
        }
        b"buFont" => {
            if ls.bu_font.is_none() {
                ls.bu_font = attr_of(e, b"typeface").filter(|t| !t.is_empty());
            }
        }
        b"buSzPct" => ls.bu_size_pct = attr_of(e, b"val").and_then(|s| s.parse().ok()),
        _ => {}
    }
}

// ---- 列表样式 --------------------------------------------------------------

/// 解析一个列表样式形元素(`a:lstStyle` / master `p:txStyles` 的一桶 /
/// `p:defaultTextStyle`):子元素 `a:lvl1pPr`…`a:lvl9pPr` 按层落位。
/// 已消费起始标签。`a:defPPr` 少见,按第 1 层的兜底并入(仅当 lvl1pPr 缺席)。
pub fn parse_list_style<R: std::io::BufRead>(reader: &mut Reader<R>) -> TextStyleLevels {
    let mut out = TextStyleLevels::default();
    let mut def_ppr: Option<TextLevelStyle> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if let Some(i) = lvl_index(&name) {
                    out.levels[i] = Some(parse_level_style(reader, &e));
                } else if name.as_slice() == b"defPPr" {
                    def_ppr = Some(parse_level_style(reader, &e));
                } else {
                    skip_element(reader, &name);
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if let Some(i) = lvl_index(&name) {
                    out.levels[i] = Some(level_style_attrs(&e));
                } else if name.as_slice() == b"defPPr" {
                    def_ppr = Some(level_style_attrs(&e));
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    if out.levels[0].is_none() {
        out.levels[0] = def_ppr;
    }
    out
}

/// `lvlNpPr` -> 0 基层号(N ∈ 1..=9)。
fn lvl_index(name: &[u8]) -> Option<usize> {
    let mid = name.strip_prefix(b"lvl")?.strip_suffix(b"pPr")?;
    if mid.len() != 1 || !mid[0].is_ascii_digit() {
        return None;
    }
    let n = (mid[0] - b'0') as usize;
    (1..=9).contains(&n).then(|| n - 1)
}
