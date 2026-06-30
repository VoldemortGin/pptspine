//! 解析单张幻灯片 XML(`ppt/slides/slideN.xml`)-> `Vec<Shape>`。
//!
//! 走 `p:cSld` > `p:spTree`,识别四类节点:
//! - `p:sp`   —— 文本框 / 自选图形(看有没有 `a:prstGeom`)
//! - `p:graphicFrame` > `a:tbl` —— 表格
//! - `p:pic`  —— 图片
//! - `p:grpSp` —— 组合(递归)
//!
//! 实现是一个**递归下降**的 quick-xml 事件遍历:每个 `parse_*` 子函数在收到对应起始标签后,
//! 一路消费到其匹配的结束标签为止,期间填充模型。容错:未知元素跳过、缺失属性 → 缺省、绝不 panic。

use std::collections::BTreeMap;

use ppt_core::geom::{Emu, Rect};
use ppt_core::model::{
    AutoShape, Cell, Color, Paragraph, Picture, Row, Shape, Table, TextFrame, TextRun,
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
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"sp" => {
                        if let Some(s) = parse_sp(reader) {
                            shapes.push(s);
                        }
                    }
                    b"graphicFrame" => {
                        if let Some(s) = parse_graphic_frame(reader) {
                            shapes.push(s);
                        }
                    }
                    b"pic" => {
                        if let Some(s) = parse_pic(reader, ctx) {
                            shapes.push(s);
                        }
                    }
                    b"grpSp" => {
                        let children = parse_shape_container(reader, ctx);
                        shapes.push(Shape::Group(children));
                    }
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
    shapes
}

/// 解析一个 `p:sp`(文本框或自选图形)。已消费 `<p:sp>` 起始标签。
fn parse_sp<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<Shape> {
    let mut rect: Option<Rect> = None;
    let mut geometry: Option<String> = None;
    let mut fill: Option<Color> = None;
    let mut stroke: Option<Color> = None;
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

/// `spPr`(形状属性)的解析结果。
#[derive(Default)]
struct SpPr {
    rect: Option<Rect>,
    geometry: Option<String>,
    fill: Option<Color>,
    stroke: Option<Color>,
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
                    b"ln" => pr.stroke = parse_line_color(reader).or(pr.stroke),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                // `<a:prstGeom prst="rect"/>` 也可能是自闭合。
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"prstGeom" {
                    pr.geometry = attr_of(&e, b"prst");
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

/// 解析 `a:ln`(描边),取其内 `a:solidFill` 颜色。已消费 `<a:ln>` 起始标签。
fn parse_line_color<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<Color> {
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

/// 解析 `a:p`(段落):`a:pPr`(lvl/algn)、`a:r`(run)。已消费 `<a:p>` 起始标签。
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
                    b"r" => {
                        if let Some(run) = parse_run(reader) {
                            runs.push(run);
                        }
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"pPr" {
                    level = attr_of(&e, b"lvl")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    align = attr_of(&e, b"algn");
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

/// 解析 `a:r`(文本 run):`a:rPr`(字体/字号/粗斜/颜色)+ `a:t`(文字)。已消费 `<a:r>` 起始标签。
fn parse_run<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<TextRun> {
    let mut text = String::new();
    let mut font: Option<String> = None;
    let mut size_pt: Option<f32> = None;
    let mut bold = false;
    let mut italic = false;
    let mut color: Option<Color> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"rPr" => {
                        // 属性:sz(百分之磅)、b、i。
                        size_pt = attr_of(&e, b"sz")
                            .and_then(|s| s.parse::<f32>().ok())
                            .map(|v| v / 100.0);
                        bold = bool_attr(&e, b"b");
                        italic = bool_attr(&e, b"i");
                        let pr = parse_rpr_children(reader);
                        font = pr.0.or(font);
                        color = pr.1.or(color);
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
                    size_pt = attr_of(&e, b"sz")
                        .and_then(|s| s.parse::<f32>().ok())
                        .map(|v| v / 100.0);
                    bold = bool_attr(&e, b"b");
                    italic = bool_attr(&e, b"i");
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    Some(TextRun {
        text,
        font,
        size_pt,
        bold,
        italic,
        color,
    })
}

/// 解析 `a:rPr` 的子元素:`a:latin@typeface`(字体)、`a:solidFill`(颜色)。
/// 已消费 `<a:rPr>` 起始标签(非自闭合形式)。返回 `(font, color)`。
fn parse_rpr_children<R: std::io::BufRead>(
    reader: &mut Reader<R>,
) -> (Option<String>, Option<Color>) {
    let mut font = None;
    let mut color = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"latin" => {
                        font = attr_of(&e, b"typeface").or(font);
                        skip_element(reader, &name);
                    }
                    b"solidFill" => color = parse_solid_fill(reader).or(color),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"latin" {
                    font = attr_of(&e, b"typeface").or(font);
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (font, color)
}

/// 解析 `p:graphicFrame`:其内 `a:graphic` > `a:graphicData` > `a:tbl` -> 表格。
/// 已消费 `<p:graphicFrame>` 起始标签。同时尝试抓取 frame 的 `p:xfrm`。
fn parse_graphic_frame<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<Shape> {
    let mut rect: Option<Rect> = None;
    let mut table: Option<Table> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"xfrm" => rect = parse_xfrm(reader).or(rect),
                    b"tbl" => {
                        table = Some(parse_table(reader, rect));
                    }
                    // graphic / graphicData 只是容器:不要 skip,继续往里走。
                    b"graphic" | b"graphicData" => {}
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
    // rect 可能在 tbl 之后才出现极少见;若 table 已建好但 rect 后到,这里补一下。
    table.map(|mut t| {
        if t.rect.is_none() {
            t.rect = rect;
        }
        Shape::Table(t)
    })
}

/// 解析 `a:tbl` -> `Table`。已消费 `<a:tbl>` 起始标签。
fn parse_table<R: std::io::BufRead>(reader: &mut Reader<R>, rect: Option<Rect>) -> Table {
    let mut rows = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"tr" {
                    let height = attr_of(&e, b"h").and_then(|s| s.parse().ok());
                    let cells = parse_table_row(reader);
                    rows.push(Row { cells, height });
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
    Table { rect, rows }
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
