//! 解析幻灯片形部件 XML(slide / slideLayout / slideMaster 共用同一 spTree 结构)
//! -> [`PartData`](形状 + 颜色映射 + master 文本样式)。
//!
//! 走 `p:cSld` > `p:spTree`,识别这些节点:
//! - `p:sp`   —— 文本框 / 自选图形(看有没有 `a:prstGeom`);占位符 `p:ph` / 列表样式
//!   `a:lstStyle` / 形状样式 `p:style` 一并捕获(B-8/B-9 继承链)
//! - `p:graphicFrame` > `a:tbl` —— 表格;非表格内容(图表 / SmartArt / OLE)降级为占位
//! - `p:pic`  —— 图片
//! - `p:grpSp` —— 组合(递归)
//! - `p:cxnSp` —— 连接线
//! - `mc:AlternateContent` —— 按锁定策略降入 `mc:Fallback`(跳过 `mc:Choice`)
//!
//! 部件级还捕获:`p:clrMap`(master)、`p:clrMapOvr`(slide/layout 的
//! `a:overrideClrMapping`)、`p:txStyles`(master 三桶文本样式)。
//!
//! 实现是一个**递归下降**的 quick-xml 事件遍历:每个 `parse_*` 子函数在收到对应起始标签后,
//! 一路消费到其匹配的结束标签为止,期间填充模型。容错:未知元素跳过、缺失属性 → 缺省、绝不 panic。

use std::collections::BTreeMap;

use ppt_core::color::ColorSpec;
use ppt_core::geom::{Emu, Rect};
use ppt_core::model::{
    AutoShape, Cell, Connector, GraphicPlaceholder, Paragraph, Picture, Row, RunKind, Shape,
    Stroke, Table, TextFrame, TextRun,
};
use ppt_core::style::{
    FontRef, PlaceholderRef, RunStyle, ShapeStyle, StyleMatrixRef, TextStyleLevels, TxStyles,
};
use ppt_core::theme::ClrMap;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::text_style::{
    level_style_attrs, parse_color_in, parse_level_style, parse_list_style, parse_run_style,
    parse_solid_fill, run_style_attrs,
};
use super::Relationship;
use super::{attr_of, attr_string, bool_attr, local_name, parse_rels, read_text, skip_element};

/// `p:txBody` 的解析结果:段落 + 自带 `a:lstStyle`。
#[derive(Debug, Clone, Default)]
struct TxBodyData {
    paragraphs: Vec<Paragraph>,
    list_style: Option<TextStyleLevels>,
}

/// 一个形部件(slide / slideLayout / slideMaster)的解析结果。
#[derive(Debug, Clone, Default)]
pub struct PartData {
    /// `p:spTree` 的顶层形状,按文档顺序。
    pub shapes: Vec<Shape>,
    /// `p:clrMap`(仅 slideMaster 有)。
    pub clr_map: Option<ClrMap>,
    /// `p:clrMapOvr > a:overrideClrMapping`(slide / layout;`a:masterClrMapping`
    /// 或缺失 → `None` = 沿用上级映射)。
    pub clr_map_ovr: Option<ClrMap>,
    /// `p:txStyles`(仅 slideMaster 有):title / body / other 三桶。
    pub tx_styles: Option<TxStyles>,
}

/// 解析一个形部件。`rels_xml` 是该部件的 `.rels` 文本(用于把图片 `r:embed` 映射到
/// media 名);`media_index` 是 `裸文件名 -> 字节长度`,用于回填 `image_bytes_len`。
pub fn parse_part(
    xml: &str,
    rels_xml: Option<&str>,
    media_index: &BTreeMap<String, usize>,
) -> PartData {
    let rels = rels_xml.map(parse_rels).unwrap_or_default();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let ctx = Ctx {
        rels: &rels,
        media_index,
    };

    let mut out = PartData::default();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"spTree" => out.shapes = parse_shape_container(&mut reader, &ctx),
                    b"clrMap" => {
                        out.clr_map = Some(clr_map_from(&e));
                        skip_element(&mut reader, &name);
                    }
                    b"clrMapOvr" => out.clr_map_ovr = parse_clr_map_ovr(&mut reader),
                    b"txStyles" => out.tx_styles = Some(parse_tx_styles(&mut reader)),
                    // 其余容器(sld / cSld / sldMaster …)继续下钻。
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"clrMap" {
                    out.clr_map = Some(clr_map_from(&e));
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

/// 从 `p:clrMap` / `a:overrideClrMapping` 的属性建 [`ClrMap`](缺失属性按惯例缺省)。
fn clr_map_from(e: &BytesStart) -> ClrMap {
    let d = ClrMap::default();
    let g = |k: &[u8], dflt: String| attr_of(e, k).unwrap_or(dflt);
    ClrMap {
        bg1: g(b"bg1", d.bg1),
        tx1: g(b"tx1", d.tx1),
        bg2: g(b"bg2", d.bg2),
        tx2: g(b"tx2", d.tx2),
        accent1: g(b"accent1", d.accent1),
        accent2: g(b"accent2", d.accent2),
        accent3: g(b"accent3", d.accent3),
        accent4: g(b"accent4", d.accent4),
        accent5: g(b"accent5", d.accent5),
        accent6: g(b"accent6", d.accent6),
        hlink: g(b"hlink", d.hlink),
        fol_hlink: g(b"folHlink", d.fol_hlink),
    }
}

/// 解析 `p:clrMapOvr`:`a:overrideClrMapping` -> `Some`;`a:masterClrMapping` -> `None`。
/// 已消费起始标签。
fn parse_clr_map_ovr<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<ClrMap> {
    let mut ovr = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"overrideClrMapping" {
                    ovr = Some(clr_map_from(&e));
                }
            }
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"overrideClrMapping" {
                    ovr = Some(clr_map_from(&e));
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
    ovr
}

/// 解析 master 的 `p:txStyles` 三桶。已消费起始标签。
fn parse_tx_styles<R: std::io::BufRead>(reader: &mut Reader<R>) -> TxStyles {
    let mut styles = TxStyles::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"titleStyle" => styles.title = parse_list_style(reader),
                    b"bodyStyle" => styles.body = parse_list_style(reader),
                    b"otherStyle" => styles.other = parse_list_style(reader),
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
    styles
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
    let mut fill: Option<ColorSpec> = None;
    let mut stroke: Option<Stroke> = None;
    let mut placeholder: Option<PlaceholderRef> = None;
    let mut style: Option<ShapeStyle> = None;
    let mut body = TxBodyData::default();
    let mut has_txbody = false;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"nvSpPr" => placeholder = parse_nv_ph(reader).or(placeholder),
                    b"spPr" => {
                        let pr = parse_sppr(reader);
                        rect = pr.rect.or(rect);
                        geometry = pr.geometry.or(geometry);
                        fill = pr.fill.or(fill);
                        stroke = pr.stroke.or(stroke);
                    }
                    b"style" => style = Some(parse_shape_style(reader)),
                    b"txBody" => {
                        has_txbody = true;
                        body = parse_txbody(reader);
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

    let text_frame = TextFrame {
        rect,
        paragraphs: body.paragraphs,
        placeholder: placeholder.clone(),
        list_style: body.list_style,
        style: style.clone(),
    };
    // 有预设几何 / 填充 / 描边 => 当作自选图形;否则当作纯文本框。
    if geometry.is_some() || fill.is_some() || stroke.is_some() {
        // 段落非空,或带 lstStyle(layout/master 占位符常态——继承链需要),才保留文字体。
        let text = if has_txbody
            && (!text_frame.paragraphs.is_empty() || text_frame.list_style.is_some())
        {
            Some(Box::new(text_frame))
        } else {
            None
        };
        Some(Shape::Auto(AutoShape {
            rect,
            geometry,
            fill,
            stroke,
            text,
            placeholder,
            style,
        }))
    } else {
        Some(Shape::TextBox(text_frame))
    }
}

/// 在 `p:nvSpPr` / `p:nvPicPr` 等非可视属性容器里找 `p:ph`(占位符标识)。
/// 已消费容器起始标签,消费到其结束标签。
fn parse_nv_ph<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<PlaceholderRef> {
    let mut ph = None;
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                if ph.is_none() && local_name(e.name().as_ref()) == b"ph" {
                    ph = Some(ph_from(&e));
                }
            }
            Ok(Event::Empty(e)) => {
                if ph.is_none() && local_name(e.name().as_ref()) == b"ph" {
                    ph = Some(ph_from(&e));
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
    ph
}

/// 从 `<p:ph>` 的属性建 [`PlaceholderRef`]。
fn ph_from(e: &BytesStart) -> PlaceholderRef {
    PlaceholderRef {
        kind: attr_of(e, b"type"),
        idx: attr_of(e, b"idx").and_then(|s| s.parse().ok()),
    }
}

/// 解析 `p:style`(主题索引式形状样式):`a:fillRef` / `a:lnRef` / `a:fontRef`。
/// 已消费起始标签。
fn parse_shape_style<R: std::io::BufRead>(reader: &mut Reader<R>) -> ShapeStyle {
    let mut style = ShapeStyle::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"fillRef" => {
                        let idx = ref_idx(&e);
                        let color = parse_color_in(reader);
                        style.fill_ref = Some(StyleMatrixRef { idx, color });
                    }
                    b"lnRef" => {
                        let idx = ref_idx(&e);
                        let color = parse_color_in(reader);
                        style.ln_ref = Some(StyleMatrixRef { idx, color });
                    }
                    b"fontRef" => {
                        let idx = attr_of(&e, b"idx").unwrap_or_default();
                        let color = parse_color_in(reader);
                        style.font_ref = Some(FontRef { idx, color });
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"fillRef" => {
                        style.fill_ref = Some(StyleMatrixRef {
                            idx: ref_idx(&e),
                            color: None,
                        })
                    }
                    b"lnRef" => {
                        style.ln_ref = Some(StyleMatrixRef {
                            idx: ref_idx(&e),
                            color: None,
                        })
                    }
                    b"fontRef" => {
                        style.font_ref = Some(FontRef {
                            idx: attr_of(&e, b"idx").unwrap_or_default(),
                            color: None,
                        })
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
    style
}

/// `a:fillRef@idx` / `a:lnRef@idx`(1 基;缺失 / 非法记 0 = 无引用)。
fn ref_idx(e: &BytesStart) -> u32 {
    attr_of(e, b"idx").and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// 解析一个 `p:cxnSp`(连接线):`spPr`(几何 / 填充 / 描边)+ `p:style`(主题线色),
/// 没有文字体。已消费 `<p:cxnSp>` 起始标签。即使属性齐缺也保留形状(信息无损、绝不静默丢弃)。
fn parse_cxn_sp<R: std::io::BufRead>(reader: &mut Reader<R>) -> Shape {
    let mut pr = SpPr::default();
    let mut style: Option<ShapeStyle> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"spPr" => {
                        let got = parse_sppr(reader);
                        pr.rect = got.rect.or(pr.rect);
                        pr.geometry = got.geometry.or(pr.geometry);
                        pr.fill = got.fill.or(pr.fill);
                        pr.stroke = got.stroke.or(pr.stroke);
                    }
                    b"style" => style = Some(parse_shape_style(reader)),
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
    Shape::Connector(Connector {
        rect: pr.rect,
        geometry: pr.geometry,
        fill: pr.fill,
        stroke: pr.stroke,
        style,
    })
}

/// `spPr`(形状属性)的解析结果。
#[derive(Default)]
struct SpPr {
    rect: Option<Rect>,
    geometry: Option<String>,
    fill: Option<ColorSpec>,
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

/// 解析 `a:ln`(描边):自身 `@w` 线宽 + 其内 `a:solidFill` 颜色 + `a:prstDash@val`
/// 虚线预设。已消费 `<a:ln>` 起始标签;`start` 是该起始标签(读取 `w`)。
/// 颜色 / 线宽 / 虚线全缺时返回 `None`(与旧行为一致:空 `a:ln` 不产生描边)。
pub(crate) fn parse_ln<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    start: &BytesStart,
) -> Option<Stroke> {
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
    color: Option<ColorSpec>,
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

/// 解析 `p:txBody` -> 段落序列 + 自带列表样式。已消费 `<p:txBody>` 起始标签。
fn parse_txbody<R: std::io::BufRead>(reader: &mut Reader<R>) -> TxBodyData {
    let mut body = TxBodyData::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"p" => body.paragraphs.push(parse_paragraph(reader)),
                    b"lstStyle" => {
                        let ls = parse_list_style(reader);
                        if !ls.is_empty() {
                            body.list_style = Some(ls);
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
    body
}

/// 解析 `a:p`(段落):`a:pPr`(完整段落属性)、`a:r`(run)、`a:br`(段内硬换行)、
/// `a:fld`(字段,如页码/日期)。已消费 `<a:p>` 起始标签。
fn parse_paragraph<R: std::io::BufRead>(reader: &mut Reader<R>) -> Paragraph {
    let mut para = Paragraph::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"pPr" => {
                        para.level = ppr_level(&e);
                        para.props = parse_level_style(reader, &e);
                        para.align = para.props.align.clone();
                    }
                    b"r" => para.runs.push(parse_run_like(reader, RunKind::Text)),
                    b"br" => {
                        // `<a:br>` 可带 `a:rPr` 子元素,整体消费掉;换行本身无文字样式语义。
                        skip_element(reader, &name);
                        para.runs.push(break_run());
                    }
                    b"fld" => {
                        let field_type = attr_of(&e, b"type");
                        para.runs
                            .push(parse_run_like(reader, RunKind::Field { field_type }));
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"pPr" => {
                        para.level = ppr_level(&e);
                        para.props = level_style_attrs(&e);
                        para.align = para.props.align.clone();
                    }
                    b"br" => para.runs.push(break_run()),
                    b"fld" => {
                        // 自闭合字段:无缓存文本,仍保留字段类型(信息无损)。
                        para.runs.push(TextRun {
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
    para
}

/// `a:pPr@lvl`(缺省 0)。
fn ppr_level(e: &BytesStart) -> u8 {
    attr_of(e, b"lvl").and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// 一个段内硬换行 run(`a:br`):`text` 固定 `"\n"`,使拼接文字自然还原换行。
fn break_run() -> TextRun {
    TextRun {
        text: "\n".to_string(),
        kind: RunKind::Break,
        ..TextRun::default()
    }
}

/// 解析一个 run 形态的元素(`a:r` 或 `a:fld`):`a:rPr`(样式)+ `a:t`(文字)。
/// 已消费其起始标签;`kind` 标记 run 种类(`a:fld` 的 `text` 是文档缓存的已渲染文本)。
fn parse_run_like<R: std::io::BufRead>(reader: &mut Reader<R>, kind: RunKind) -> TextRun {
    let mut text = String::new();
    let mut rs = RunStyle::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"rPr" => rs = parse_run_style(reader, &e),
                    b"t" => {
                        text.push_str(&read_text(reader));
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"rPr" {
                    rs = run_style_attrs(&e);
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
        font: rs.font,
        ea_font: rs.ea_font,
        cs_font: rs.cs_font,
        size_pt: rs.size_pt,
        bold: rs.bold,
        italic: rs.italic,
        underline: rs.underline,
        strike: rs.strike,
        color: rs.color,
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
                    b"txBody" => cell.paragraphs = parse_txbody(reader).paragraphs,
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
fn parse_tcpr_fill<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<ColorSpec> {
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

/// 解析 `p:pic`(图片):`p:spPr`(位置)+ `a:blip@r:embed`(rel id)+ 占位符标识。
/// 已消费 `<p:pic>` 起始标签。
fn parse_pic<R: std::io::BufRead>(reader: &mut Reader<R>, ctx: &Ctx) -> Option<Shape> {
    let mut rect: Option<Rect> = None;
    let mut rel_id = String::new();
    let mut placeholder: Option<PlaceholderRef> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"nvPicPr" => placeholder = parse_nv_ph(reader).or(placeholder),
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
        placeholder,
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
