//! 解析单张幻灯片 XML(`ppt/slides/slideN.xml`)-> `Vec<Shape>`。
//!
//! 走 `p:cSld` > `p:spTree`,识别这些节点:
//! - `p:sp`   —— 文本框 / 自选图形(看有没有 `a:prstGeom`)
//! - `p:graphicFrame` > `a:tbl` —— 表格;非表格内容(图表 / SmartArt / OLE)降级为占位
//! - `p:pic`  —— 图片
//! - `p:grpSp` —— 组合(递归)
//! - `p:cxnSp` —— 连接线
//! - `mc:AlternateContent` —— 按锁定策略降入 `mc:Fallback`(跳过 `mc:Choice`)
//!
//! 实现是一个**递归下降**的 quick-xml 事件遍历:每个 `parse_*` 子函数在收到对应起始标签后,
//! 一路消费到其匹配的结束标签为止,期间填充模型。容错:未知元素跳过、缺失属性 → 缺省、绝不 panic。

use std::collections::BTreeMap;

use ppt_core::geom::{Emu, Rect};
use ppt_core::model::{
    AutoShape, Cell, Color, Connector, GraphicPlaceholder, Paragraph, Picture, Row, RunKind, Shape,
    Stroke, Table, TextFrame, TextRun,
};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::{attr_string, local_name, parse_rels, Relationship};

/// 解析一张幻灯片。`rels_xml` 是该 slide 的 `.rels` 文本(用于把图片 `r:embed` 映射到 media 名);
/// `media_index` 是 `裸文件名 -> 字节长度`,用于回填 `image_bytes_len`。
pub fn parse(
    xml: &str,
    rels_xml: Option<&str>,
    media_index: &BTreeMap<String, usize>,
) -> Vec<Shape> {
    let rels = rels_xml.map(parse_rels).unwrap_or_default();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let ctx = Ctx {
        rels: &rels,
        media_index,
    };

    // 先定位到 spTree,再解析其直接子形状。
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == b"spTree" {
                    return parse_shape_container(&mut reader, &ctx);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    Vec::new()
}

/// 解析期的只读上下文。
struct Ctx<'a> {
    rels: &'a BTreeMap<String, Relationship>,
    media_index: &'a BTreeMap<String, usize>,
}

/// 解析一个形状容器(`p:spTree` 或 `p:grpSp`)的直接子形状,直到容器结束标签。
/// 假定 reader 已经消费了容器的起始标签。
fn parse_shape_container<R: std::io::BufRead>(reader: &mut Reader<R>, ctx: &Ctx) -> Vec<Shape> {
    let mut shapes = Vec::new();
    parse_shapes_into(reader, ctx, &mut shapes);
    shapes
}

/// 把一个容器的直接子形状解析后追加到 `out`,直到容器结束标签。
/// `p:spTree` / `p:grpSp` / `mc:Fallback` 共用这一份分发逻辑。
fn parse_shapes_into<R: std::io::BufRead>(reader: &mut Reader<R>, ctx: &Ctx, out: &mut Vec<Shape>) {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"sp" => {
                        if let Some(s) = parse_sp(reader) {
                            out.push(s);
                        }
                    }
                    b"graphicFrame" => {
                        if let Some(s) = parse_graphic_frame(reader) {
                            out.push(s);
                        }
                    }
                    b"pic" => {
                        if let Some(s) = parse_pic(reader, ctx) {
                            out.push(s);
                        }
                    }
                    b"cxnSp" => out.push(parse_cxn_sp(reader)),
                    b"grpSp" => {
                        let children = parse_shape_container(reader, ctx);
                        out.push(Shape::Group(children));
                    }
                    b"AlternateContent" => parse_alternate_content(reader, ctx, out),
                    // 其它直接子元素(grpSpPr / nvGrpSpPr 等)整体跳过。
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::End(_)) => break, // 容器结束。
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
}

/// 解析 `mc:AlternateContent`:按锁定策略降入 `mc:Fallback`(兼容表示),把其中的形状
/// 追加到 `out`;所有 `mc:Choice`(及其它子元素)整体跳过。已消费起始标签。
fn parse_alternate_content<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    ctx: &Ctx,
    out: &mut Vec<Shape>,
) {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"Fallback" {
                    parse_shapes_into(reader, ctx, out);
                } else {
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
}

/// 解析一个 `p:sp`(文本框或自选图形)。已消费 `<p:sp>` 起始标签。
fn parse_sp<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<Shape> {
    let mut rect: Option<Rect> = None;
    let mut geometry: Option<String> = None;
    let mut fill: Option<Color> = None;
    let mut stroke: Option<Stroke> = None;
    let mut paragraphs: Vec<Paragraph> = Vec::new();
    let mut has_txbody = false;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"spPr" => {
                        let pr = parse_sppr(reader);
                        rect = pr.rect.or(rect);
                        geometry = pr.geometry.or(geometry);
                        fill = pr.fill.or(fill);
                        stroke = pr.stroke.or(stroke);
                    }
                    b"txBody" => {
                        has_txbody = true;
                        paragraphs = parse_txbody(reader);
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

    let text_frame = TextFrame { rect, paragraphs };
    // 有预设几何 / 填充 / 描边 => 当作自选图形;否则当作纯文本框。
    if geometry.is_some() || fill.is_some() || stroke.is_some() {
        let text = if has_txbody && !text_frame.paragraphs.is_empty() {
            Some(text_frame)
        } else {
            None
        };
        Some(Shape::Auto(AutoShape {
            rect,
            geometry,
            fill,
            stroke,
            text,
        }))
    } else {
        Some(Shape::TextBox(text_frame))
    }
}

/// 解析一个 `p:cxnSp`(连接线):只有 `spPr`(几何 / 填充 / 描边),没有文字体。
/// 已消费 `<p:cxnSp>` 起始标签。即使属性齐缺也保留形状(信息无损、绝不静默丢弃)。
fn parse_cxn_sp<R: std::io::BufRead>(reader: &mut Reader<R>) -> Shape {
    let mut pr = SpPr::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"spPr" {
                    let got = parse_sppr(reader);
                    pr.rect = got.rect.or(pr.rect);
                    pr.geometry = got.geometry.or(pr.geometry);
                    pr.fill = got.fill.or(pr.fill);
                    pr.stroke = got.stroke.or(pr.stroke);
                } else {
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
    Shape::Connector(Connector {
        rect: pr.rect,
        geometry: pr.geometry,
        fill: pr.fill,
        stroke: pr.stroke,
    })
}

/// `spPr`(形状属性)的解析结果。
#[derive(Default)]
struct SpPr {
    rect: Option<Rect>,
    geometry: Option<String>,
    fill: Option<Color>,
    stroke: Option<Stroke>,
}

/// 解析 `a:spPr`:`a:xfrm`(位置尺寸)、`a:prstGeom`(几何名)、`a:solidFill`(填充)、
/// `a:ln`(描边,其内可再有 `a:solidFill`)。已消费 `<*:spPr>` 起始标签。
fn parse_sppr<R: std::io::BufRead>(reader: &mut Reader<R>) -> SpPr {
    let mut pr = SpPr::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"xfrm" => pr.rect = parse_xfrm(reader),
                    b"prstGeom" => {
                        pr.geometry = attr_of(&e, b"prst");
                        skip_element(reader, &name);
                    }
                    b"solidFill" => pr.fill = parse_solid_fill(reader).or(pr.fill),
                    b"ln" => pr.stroke = parse_ln(reader, &e).or(pr.stroke),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                // `<a:prstGeom prst="rect"/>` / `<a:ln w="…"/>` 也可能是自闭合。
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"prstGeom" => pr.geometry = attr_of(&e, b"prst"),
                    b"ln" => pr.stroke = stroke_if_any(None, ln_width(&e), None).or(pr.stroke),
                    _ => {}
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    pr
}

/// 解析 `a:xfrm` 内的 `a:off`(x/y)与 `a:ext`(cx/cy)-> `Rect`。已消费 `<a:xfrm>` 起始标签。
fn parse_xfrm<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<Rect> {
    let mut x: Option<Emu> = None;
    let mut y: Option<Emu> = None;
    let mut w: Option<Emu> = None;
    let mut h: Option<Emu> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"off" => {
                        x = attr_of(&e, b"x").and_then(|s| s.parse().ok());
                        y = attr_of(&e, b"y").and_then(|s| s.parse().ok());
                    }
                    b"ext" => {
                        w = attr_of(&e, b"cx").and_then(|s| s.parse().ok());
                        h = attr_of(&e, b"cy").and_then(|s| s.parse().ok());
                    }
                    _ => {}
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    match (x, y, w, h) {
        (Some(x), Some(y), Some(w), Some(h)) => Some(Rect::new(x, y, w, h)),
        _ => None,
    }
}

/// 解析一个 `a:solidFill` 里的 `a:srgbClr@val` -> 颜色。已消费 `<a:solidFill>` 起始标签。
/// `srgbClr` 可能是自闭合也可能带子元素(alpha 等);两种形式都按本地名匹配取 `val`。
fn parse_solid_fill<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<Color> {
    let mut color = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"srgbClr" {
                    if let Some(hex) = attr_of(&e, b"val") {
                        color = Color::from_hex(&hex);
                    }
                }
            }
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"srgbClr" {
                    if let Some(hex) = attr_of(&e, b"val") {
                        color = Color::from_hex(&hex);
                    }
                }
                // 带子元素的形式:消费到它的结束标签。
                skip_element(reader, &name);
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    color
}

/// 解析 `a:ln`(描边):自身 `@w` 线宽 + 其内 `a:solidFill` 颜色 + `a:prstDash@val`
/// 虚线预设。已消费 `<a:ln>` 起始标签;`start` 是该起始标签(读取 `w`)。
/// 颜色 / 线宽 / 虚线全缺时返回 `None`(与旧行为一致:空 `a:ln` 不产生描边)。
fn parse_ln<R: std::io::BufRead>(reader: &mut Reader<R>, start: &BytesStart) -> Option<Stroke> {
    let width_emu = ln_width(start);
    let mut color = None;
    let mut dash = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"solidFill" => color = parse_solid_fill(reader).or(color),
                    b"prstDash" => {
                        dash = attr_of(&e, b"val").or(dash);
                        skip_element(reader, &name);
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                // `<a:prstDash val="dash"/>` 通常是自闭合。
                if local_name(e.name().as_ref()) == b"prstDash" {
                    dash = attr_of(&e, b"val").or(dash);
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    stroke_if_any(color, width_emu, dash)
}

/// `a:ln@w`(EMU 线宽)。
fn ln_width(e: &BytesStart) -> Option<Emu> {
    attr_of(e, b"w").and_then(|s| s.parse().ok())
}

/// 颜色 / 线宽 / 虚线至少有一项时建 [`Stroke`],否则 `None`。
fn stroke_if_any(
    color: Option<Color>,
    width_emu: Option<Emu>,
    dash: Option<String>,
) -> Option<Stroke> {
    if color.is_none() && width_emu.is_none() && dash.is_none() {
        return None;
    }
    Some(Stroke {
        color,
        width_emu,
        dash,
    })
}

/// 解析 `p:txBody` -> 段落序列。已消费 `<p:txBody>` 起始标签。
fn parse_txbody<R: std::io::BufRead>(reader: &mut Reader<R>) -> Vec<Paragraph> {
    let mut paras = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"p" {
                    paras.push(parse_paragraph(reader));
                } else {
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
    paras
}

/// 解析 `a:p`(段落):`a:pPr`(lvl/algn)、`a:r`(run)、`a:br`(段内硬换行)、
/// `a:fld`(字段,如页码/日期)。已消费 `<a:p>` 起始标签。
fn parse_paragraph<R: std::io::BufRead>(reader: &mut Reader<R>) -> Paragraph {
    let mut runs = Vec::new();
    let mut level: u8 = 0;
    let mut align: Option<String> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"pPr" => {
                        level = attr_of(&e, b"lvl")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
                        align = attr_of(&e, b"algn");
                        skip_element(reader, &name);
                    }
                    b"r" => runs.push(parse_run_like(reader, RunKind::Text)),
                    b"br" => {
                        // `<a:br>` 可带 `a:rPr` 子元素,整体消费掉;换行本身无文字样式语义。
                        skip_element(reader, &name);
                        runs.push(break_run());
                    }
                    b"fld" => {
                        let field_type = attr_of(&e, b"type");
                        runs.push(parse_run_like(reader, RunKind::Field { field_type }));
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"pPr" => {
                        level = attr_of(&e, b"lvl")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
                        align = attr_of(&e, b"algn");
                    }
                    b"br" => runs.push(break_run()),
                    b"fld" => {
                        // 自闭合字段:无缓存文本,仍保留字段类型(信息无损)。
                        runs.push(TextRun {
                            kind: RunKind::Field {
                                field_type: attr_of(&e, b"type"),
                            },
                            ..TextRun::default()
                        });
                    }
                    _ => {}
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    Paragraph { runs, level, align }
}

/// 一个段内硬换行 run(`a:br`):`text` 固定 `"\n"`,使拼接文字自然还原换行。
fn break_run() -> TextRun {
    TextRun {
        text: "\n".to_string(),
        kind: RunKind::Break,
        ..TextRun::default()
    }
}

/// `a:rPr` 属性上的 run 样式:sz(百分之磅)/ b / i / u / strike。
#[derive(Default)]
struct RunAttrs {
    size_pt: Option<f32>,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

impl RunAttrs {
    fn from(e: &BytesStart) -> Self {
        RunAttrs {
            size_pt: attr_of(e, b"sz")
                .and_then(|s| s.parse::<f32>().ok())
                .map(|v| v / 100.0),
            bold: bool_attr(e, b"b"),
            italic: bool_attr(e, b"i"),
            // `u="none"` / `strike="noStrike"` 是显式关闭,不算开启。
            underline: attr_of(e, b"u").is_some_and(|v| v != "none"),
            strike: attr_of(e, b"strike").is_some_and(|v| v != "noStrike"),
        }
    }
}

/// 解析一个 run 形态的元素(`a:r` 或 `a:fld`):`a:rPr`(样式)+ `a:t`(文字)。
/// 已消费其起始标签;`kind` 标记 run 种类(`a:fld` 的 `text` 是文档缓存的已渲染文本)。
fn parse_run_like<R: std::io::BufRead>(reader: &mut Reader<R>, kind: RunKind) -> TextRun {
    let mut text = String::new();
    let mut attrs = RunAttrs::default();
    let mut fonts = RunFonts::default();
    let mut color: Option<Color> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"rPr" => {
                        attrs = RunAttrs::from(&e);
                        let (f, c) = parse_rpr_children(reader);
                        fonts = f;
                        color = c.or(color);
                    }
                    b"t" => {
                        text.push_str(&read_text(reader));
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"rPr" {
                    attrs = RunAttrs::from(&e);
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    TextRun {
        text,
        kind,
        font: fonts.latin,
        ea_font: fonts.ea,
        cs_font: fonts.cs,
        size_pt: attrs.size_pt,
        bold: attrs.bold,
        italic: attrs.italic,
        underline: attrs.underline,
        strike: attrs.strike,
        color,
    }
}

/// `a:rPr` 子元素里的三路字体名:latin / ea(东亚,CJK 关键)/ cs(复杂文种)。
#[derive(Default)]
struct RunFonts {
    latin: Option<String>,
    ea: Option<String>,
    cs: Option<String>,
}

/// 解析 `a:rPr` 的子元素:`a:latin`/`a:ea`/`a:cs` 的 `@typeface`(字体)、
/// `a:solidFill`(颜色)。已消费 `<a:rPr>` 起始标签(非自闭合形式)。
fn parse_rpr_children<R: std::io::BufRead>(reader: &mut Reader<R>) -> (RunFonts, Option<Color>) {
    let mut fonts = RunFonts::default();
    let mut color = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"latin" | b"ea" | b"cs" => {
                        set_typeface(&mut fonts, &name, &e);
                        skip_element(reader, &name);
                    }
                    b"solidFill" => color = parse_solid_fill(reader).or(color),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if matches!(name.as_slice(), b"latin" | b"ea" | b"cs") {
                    set_typeface(&mut fonts, &name, &e);
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (fonts, color)
}

/// 把 `a:latin`/`a:ea`/`a:cs` 的 `@typeface` 填进对应槽位(已有值不覆盖)。
fn set_typeface(fonts: &mut RunFonts, name: &[u8], e: &BytesStart) {
    let slot = match name {
        b"latin" => &mut fonts.latin,
        b"ea" => &mut fonts.ea,
        _ => &mut fonts.cs,
    };
    if slot.is_none() {
        *slot = attr_of(e, b"typeface");
    }
}

/// 解析 `p:graphicFrame`:其内 `a:graphic` > `a:graphicData` > `a:tbl` -> 表格;
/// 非表格内容(图表 / SmartArt / OLE 等)降级为 [`Shape::Placeholder`],至少保住外框
/// 矩形与 `graphicData@uri`(渲染侧据此画占位框 + 告警)。已消费起始标签。
fn parse_graphic_frame<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<Shape> {
    let mut rect: Option<Rect> = None;
    let mut table: Option<Table> = None;
    let mut uri: Option<String> = None;
    // graphic / graphicData 是要"穿透"的容器:降入时计深,End 时消深,直到
    // `</p:graphicFrame>` 本身(depth 归零)才结束。此前不计深、见 End 就 break,
    // 会把 `</a:graphic>`/`</p:graphicFrame>` 留给上层容器误吞,静默丢掉 frame
    // 之后的所有同级形状。
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    // 这些分支各自消费到自己的结束标签,不影响 depth。
                    b"xfrm" => rect = parse_xfrm(reader).or(rect),
                    b"tbl" => {
                        table = Some(parse_table(reader, rect));
                    }
                    // graphic / graphicData 只是容器:不要 skip,继续往里走。
                    b"graphic" => depth += 1,
                    b"graphicData" => {
                        uri = attr_of(&e, b"uri").or(uri);
                        depth += 1;
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"graphicData" {
                    uri = attr_of(&e, b"uri").or(uri);
                }
            }
            Ok(Event::End(_)) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    match table {
        Some(mut t) => {
            // rect 可能在 tbl 之后才出现极少见;若 table 已建好但 rect 后到,这里补一下。
            if t.rect.is_none() {
                t.rect = rect;
            }
            Some(Shape::Table(t))
        }
        None => Some(Shape::Placeholder(GraphicPlaceholder { rect, kind: uri })),
    }
}

/// 解析 `a:tbl` -> `Table`(`a:tblGrid` 列宽 + `a:tr` 行)。已消费 `<a:tbl>` 起始标签。
fn parse_table<R: std::io::BufRead>(reader: &mut Reader<R>, rect: Option<Rect>) -> Table {
    let mut col_widths = Vec::new();
    let mut rows = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"tblGrid" => col_widths = parse_tbl_grid(reader),
                    b"tr" => {
                        let height = attr_of(&e, b"h").and_then(|s| s.parse().ok());
                        let cells = parse_table_row(reader);
                        rows.push(Row { cells, height });
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
    Table {
        rect,
        col_widths,
        rows,
    }
}

/// 解析 `a:tblGrid` -> 各列宽(EMU,`a:gridCol@w`,按文档顺序)。已消费起始标签。
/// 缺失 / 非法的 `w` 记 0,保持列数与文档一致(绝对定位由渲染侧容错)。
fn parse_tbl_grid<R: std::io::BufRead>(reader: &mut Reader<R>) -> Vec<Emu> {
    let mut widths = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"gridCol" {
                    widths.push(grid_col_width(&e));
                }
            }
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"gridCol" {
                    widths.push(grid_col_width(&e));
                }
                // gridCol 可带 extLst 子元素;其余未知元素同样整体跳过。
                skip_element(reader, &name);
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    widths
}

/// `a:gridCol@w`(EMU 列宽);缺失 / 非法记 0。
fn grid_col_width(e: &BytesStart) -> Emu {
    attr_of(e, b"w").and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// 解析 `a:tr`(表格行)-> 单元格序列。已消费 `<a:tr>` 起始标签。
fn parse_table_row<R: std::io::BufRead>(reader: &mut Reader<R>) -> Vec<Cell> {
    let mut cells = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"tc" {
                    cells.push(parse_table_cell(reader, &e));
                } else {
                    skip_element(reader, &name);
                }
            }
            Ok(Event::Empty(e)) => {
                // 自闭合的 `<a:tc .../>`(纯合并延续格)。
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"tc" {
                    cells.push(cell_skeleton(&e));
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    cells
}

/// 从 `<a:tc>` 的属性建一个合并信息已填、内容待填的单元格骨架。
fn cell_skeleton(e: &BytesStart) -> Cell {
    let col_span = attr_of(e, b"gridSpan")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let row_span = attr_of(e, b"rowSpan")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let merged = bool_attr(e, b"hMerge") || bool_attr(e, b"vMerge");
    Cell {
        paragraphs: Vec::new(),
        col_span,
        row_span,
        fill: None,
        merged,
    }
}

/// 解析 `a:tc`(单元格):`a:txBody`(文字)+ `a:tcPr`(填充)。已消费 `<a:tc>` 起始标签;
/// `start` 是该起始标签(用于读取合并属性)。
fn parse_table_cell<R: std::io::BufRead>(reader: &mut Reader<R>, start: &BytesStart) -> Cell {
    let mut cell = cell_skeleton(start);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"txBody" => cell.paragraphs = parse_txbody(reader),
                    b"tcPr" => cell.fill = parse_tcpr_fill(reader).or(cell.fill),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(_)) => {}
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    cell
}

/// 解析 `a:tcPr` 里的 `a:solidFill` 颜色。已消费 `<a:tcPr>` 起始标签。
fn parse_tcpr_fill<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<Color> {
    let mut color = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"solidFill" {
                    color = parse_solid_fill(reader).or(color);
                } else {
                    skip_element(reader, &name);
                }
            }
            Ok(Event::Empty(_)) => {}
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    color
}

/// 解析 `p:pic`(图片):`p:spPr`(位置)+ `a:blip@r:embed`(rel id)。已消费 `<p:pic>` 起始标签。
fn parse_pic<R: std::io::BufRead>(reader: &mut Reader<R>, ctx: &Ctx) -> Option<Shape> {
    let mut rect: Option<Rect> = None;
    let mut rel_id = String::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"spPr" => {
                        let pr = parse_sppr(reader);
                        rect = pr.rect.or(rect);
                    }
                    b"blipFill" => {
                        if let Some(rid) = parse_blip_fill(reader) {
                            rel_id = rid;
                        }
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

    // 经 rels 把 rel_id 映射到 media 裸文件名。
    let media_name = ctx.rels.get(&rel_id).map(|r| {
        super::normalize_target(&r.target)
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string()
    });
    let image_bytes_len = media_name
        .as_ref()
        .and_then(|n| ctx.media_index.get(n).copied())
        .unwrap_or(0);

    Some(Shape::Picture(Picture {
        rect,
        rel_id,
        media_name,
        image_bytes_len,
    }))
}

/// 解析 `p:blipFill` 内的 `a:blip@r:embed` -> rel id。已消费 `<p:blipFill>` 起始标签。
fn parse_blip_fill<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<String> {
    let mut rid = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"blip" {
                    // `r:embed` 属性。
                    for attr in e.attributes().flatten() {
                        if local_name(attr.key.as_ref()) == b"embed" {
                            rid = Some(attr_string(&attr));
                        }
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
    rid
}

// ---- 通用小工具 ----------------------------------------------------------

/// 取元素的某个属性值(按本地名匹配,忽略命名空间前缀)。
fn attr_of(e: &BytesStart, key: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if local_name(attr.key.as_ref()) == key {
            return Some(attr_string(&attr));
        }
    }
    None
}

/// 读取一个 OOXML 布尔属性。OOXML 里 `b="1"` / `b="true"` 为真;缺失为假。
fn bool_attr(e: &BytesStart, key: &[u8]) -> bool {
    match attr_of(e, key) {
        Some(v) => v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("on"),
        None => false,
    }
}

/// 读取当前已打开元素的纯文本内容,直到其结束标签。已消费该元素的起始标签。
fn read_text<R: std::io::BufRead>(reader: &mut Reader<R>) -> String {
    let mut out = String::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Text(t)) => {
                if let Ok(s) = t.unescape() {
                    out.push_str(&s);
                }
            }
            Ok(Event::CData(c)) => {
                out.push_str(&String::from_utf8_lossy(&c));
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

/// 跳过当前已打开元素的全部内容,直到其匹配的结束标签。已消费该元素的起始标签。
/// 通过深度计数处理同名嵌套。
fn skip_element<R: std::io::BufRead>(reader: &mut Reader<R>, _name: &[u8]) {
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(_)) => depth += 1,
            Ok(Event::End(_)) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
}
