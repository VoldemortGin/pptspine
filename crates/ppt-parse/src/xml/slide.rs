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
    AutoShape, Autofit, Background, BodyProps, Cell, CellBorders, Connector, Fill,
    GraphicPlaceholder, GroupShape, Paragraph, Picture, RelRect, Row, RunKind, Shape, Stroke,
    Table, TextFrame, TextRun, Xfrm,
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
use super::{
    attr_of, attr_string, bool_attr, local_name, ooxml_bool, parse_rels, read_text, skip_element,
};

/// `p:txBody` 的解析结果:段落 + 自带 `a:lstStyle` + `a:bodyPr`。
#[derive(Debug, Clone, Default)]
struct TxBodyData {
    paragraphs: Vec<Paragraph>,
    list_style: Option<TextStyleLevels>,
    body: BodyProps,
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
    /// `p:cSld > p:bg`(slide / layout / master 皆可有,B-10)。
    pub background: Option<Background>,
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
                    b"bg" => out.background = parse_bg(&mut reader, &ctx),
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

/// 解析 `p:bg`(B-10,§3.o):`p:bgPr`(直接填充 / 图片)或 `p:bgRef`(主题引用)。
/// 已消费 `<p:bg>` 起始标签。
fn parse_bg<R: std::io::BufRead>(reader: &mut Reader<R>, ctx: &Ctx) -> Option<Background> {
    let mut bg = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"bgPr" => bg = parse_bg_pr(reader, ctx).or(bg),
                    b"bgRef" => {
                        bg = Some(Background::Ref {
                            idx: ref_idx(&e),
                            color: parse_color_in(reader),
                        });
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"bgRef" {
                    bg = Some(Background::Ref {
                        idx: ref_idx(&e),
                        color: None,
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
    bg
}

/// 解析 `p:bgPr` 的第一个填充子元素:solidFill / gradFill / blipFill(经 rels 折
/// media 裸名)/ noFill。已消费起始标签。
fn parse_bg_pr<R: std::io::BufRead>(reader: &mut Reader<R>, ctx: &Ctx) -> Option<Background> {
    let mut bg = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"solidFill" => {
                        if let Some(spec) = parse_solid_fill(reader) {
                            bg = Some(Background::Fill(Fill::Solid(spec)));
                        }
                    }
                    b"gradFill" => {
                        bg = Some(Background::Fill(Fill::Gradient(parse_grad_fill(reader))));
                    }
                    b"blipFill" => {
                        let data = parse_blip_fill(reader);
                        bg = Some(Background::Blip {
                            media_name: data.rel_id.as_deref().and_then(|r| media_name_of(ctx, r)),
                        });
                    }
                    b"noFill" => {
                        bg = Some(Background::Fill(Fill::None));
                        skip_element(reader, &name);
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"noFill" {
                    bg = Some(Background::Fill(Fill::None));
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    bg
}

/// 解析期的只读上下文。
struct Ctx<'a> {
    rels: &'a BTreeMap<String, Relationship>,
    media_index: &'a BTreeMap<String, usize>,
}

/// 经部件 rels 把一个 `r:embed` 关系 id 折成 media 裸文件名(如 `image1.png`)。
fn media_name_of(ctx: &Ctx, rel_id: &str) -> Option<String> {
    ctx.rels.get(rel_id).map(|r| {
        super::normalize_target(&r.target)
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string()
    })
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
                if !dispatch_shape(&name, reader, ctx, out) {
                    // 其它直接子元素(grpSpPr / nvGrpSpPr 等)整体跳过。
                    skip_element(reader, &name);
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

/// 形状元素分发(`parse_shapes_into` / `parse_grp_sp` 共用):识别并解析一个形状
/// 起始标签,追加到 `out`。非形状元素返回 `false`(调用方自行跳过)。
fn dispatch_shape<R: std::io::BufRead>(
    name: &[u8],
    reader: &mut Reader<R>,
    ctx: &Ctx,
    out: &mut Vec<Shape>,
) -> bool {
    match name {
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
        b"grpSp" => out.push(Shape::Group(parse_grp_sp(reader, ctx))),
        b"AlternateContent" => parse_alternate_content(reader, ctx, out),
        _ => return false,
    }
    true
}

/// 解析一个 `p:grpSp`(组合):`p:grpSpPr > a:xfrm`(off/ext + chOff/chExt +
/// rot/flip)+ 子形状。已消费 `<p:grpSp>` 起始标签(B-5,§3.e)。
fn parse_grp_sp<R: std::io::BufRead>(reader: &mut Reader<R>, ctx: &Ctx) -> GroupShape {
    let mut group = GroupShape::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"grpSpPr" {
                    if let Some(x) = parse_grp_sppr(reader) {
                        group.rect = x.rect;
                        group.child_rect = x.child_rect;
                        group.xfrm = x.xfrm;
                    }
                } else if !dispatch_shape(&name, reader, ctx, &mut group.children) {
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
    group
}

/// 在 `p:grpSpPr` 里找 `a:xfrm`(组合变换)。已消费起始标签,消费到其结束标签。
fn parse_grp_sppr<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<XfrmData> {
    let mut xfrm = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"xfrm" {
                    xfrm = Some(parse_xfrm(reader, &e));
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
    xfrm
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
    let mut pr = SpPr::default();
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
                        let got = parse_sppr(reader);
                        pr.rect = got.rect.or(pr.rect);
                        pr.xfrm = got.xfrm;
                        pr.geometry = got.geometry.or(pr.geometry);
                        pr.adjusts = got.adjusts;
                        pr.fill = got.fill.or(pr.fill);
                        pr.stroke = got.stroke.or(pr.stroke);
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
        rect: pr.rect,
        xfrm: pr.xfrm,
        paragraphs: body.paragraphs,
        placeholder: placeholder.clone(),
        list_style: body.list_style,
        style: style.clone(),
        body: body.body,
    };
    // 有预设几何 / 实际填充 / 描边 => 当作自选图形;否则当作纯文本框
    // (孤立的显式 `noFill` 不改变分类——渲染结果与纯文本框一致)。
    let has_paint = pr.fill.as_ref().is_some_and(|f| !matches!(f, Fill::None));
    if pr.geometry.is_some() || has_paint || pr.stroke.is_some() {
        // 段落非空,或带 lstStyle(layout/master 占位符常态——继承链需要),才保留文字体。
        let text = if has_txbody
            && (!text_frame.paragraphs.is_empty() || text_frame.list_style.is_some())
        {
            Some(Box::new(text_frame))
        } else {
            None
        };
        Some(Shape::Auto(AutoShape {
            rect: pr.rect,
            xfrm: pr.xfrm,
            geometry: pr.geometry,
            adjusts: pr.adjusts,
            fill: pr.fill,
            stroke: pr.stroke,
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
                        pr.xfrm = got.xfrm;
                        pr.geometry = got.geometry.or(pr.geometry);
                        pr.adjusts = got.adjusts;
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
        xfrm: pr.xfrm,
        geometry: pr.geometry,
        adjusts: pr.adjusts,
        fill: pr.fill,
        stroke: pr.stroke,
        style,
    })
}

/// `spPr`(形状属性)的解析结果。
#[derive(Default)]
struct SpPr {
    rect: Option<Rect>,
    xfrm: Xfrm,
    geometry: Option<String>,
    adjusts: Vec<(String, i64)>,
    fill: Option<Fill>,
    stroke: Option<Stroke>,
}

/// 解析 `a:spPr`:`a:xfrm`(位置尺寸 + 旋转/翻转)、`a:prstGeom`(几何名 + avLst
/// 调整值)、填充(`a:solidFill`/`a:noFill`/`a:gradFill`/`a:blipFill`)、
/// `a:ln`(描边,其内可再有 `a:solidFill`)。已消费 `<*:spPr>` 起始标签。
fn parse_sppr<R: std::io::BufRead>(reader: &mut Reader<R>) -> SpPr {
    let mut pr = SpPr::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"xfrm" => {
                        let x = parse_xfrm(reader, &e);
                        pr.rect = x.rect;
                        pr.xfrm = x.xfrm;
                    }
                    b"prstGeom" => {
                        pr.geometry = attr_of(&e, b"prst");
                        pr.adjusts = parse_av_lst(reader);
                    }
                    b"solidFill" => {
                        if let Some(spec) = parse_solid_fill(reader) {
                            pr.fill = Some(Fill::Solid(spec));
                        }
                    }
                    b"noFill" => {
                        pr.fill = Some(Fill::None);
                        skip_element(reader, &name);
                    }
                    b"gradFill" => pr.fill = Some(Fill::Gradient(parse_grad_fill(reader))),
                    b"blipFill" => {
                        pr.fill = Some(Fill::Blip);
                        skip_element(reader, &name);
                    }
                    b"ln" => pr.stroke = parse_ln(reader, &e).or(pr.stroke),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                // `<a:prstGeom prst="rect"/>` / `<a:noFill/>` / `<a:ln w="…"/>`
                // 也可能是自闭合。
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"prstGeom" => pr.geometry = attr_of(&e, b"prst"),
                    b"noFill" => pr.fill = Some(Fill::None),
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

/// `a:xfrm` 的完整解析结果:矩形 + 旋转/翻转 + 子坐标空间(组合才有)。
#[derive(Debug, Clone, Copy, Default)]
struct XfrmData {
    rect: Option<Rect>,
    child_rect: Option<Rect>,
    xfrm: Xfrm,
}

/// 解析 `a:xfrm`:自身属性 `rot`/`flipH`/`flipV`(§3.d)+ 子元素 `a:off`/`a:ext`
/// (-> `rect`)与 `a:chOff`/`a:chExt`(-> `child_rect`,组合子坐标空间,§3.e)。
/// 已消费 `<a:xfrm>` 起始标签;`start` 是该起始标签。
fn parse_xfrm<R: std::io::BufRead>(reader: &mut Reader<R>, start: &BytesStart) -> XfrmData {
    let xfrm = Xfrm {
        rot: attr_of(start, b"rot")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        flip_h: bool_attr(start, b"flipH"),
        flip_v: bool_attr(start, b"flipV"),
    };
    let mut off: Option<(Emu, Emu)> = None;
    let mut ext: Option<(Emu, Emu)> = None;
    let mut ch_off: Option<(Emu, Emu)> = None;
    let mut ch_ext: Option<(Emu, Emu)> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"off" => off = xy_of(&e, b"x", b"y"),
                    b"ext" => ext = xy_of(&e, b"cx", b"cy"),
                    b"chOff" => ch_off = xy_of(&e, b"x", b"y"),
                    b"chExt" => ch_ext = xy_of(&e, b"cx", b"cy"),
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
    let rect_of = |o: Option<(Emu, Emu)>, e: Option<(Emu, Emu)>| match (o, e) {
        (Some((x, y)), Some((w, h))) => Some(Rect::new(x, y, w, h)),
        _ => None,
    };
    XfrmData {
        rect: rect_of(off, ext),
        child_rect: rect_of(ch_off, ch_ext),
        xfrm,
    }
}

/// 从元素属性读一对 EMU 坐标(两个都合法才算)。
fn xy_of(e: &BytesStart, kx: &[u8], ky: &[u8]) -> Option<(Emu, Emu)> {
    let x = attr_of(e, kx).and_then(|s| s.parse().ok())?;
    let y = attr_of(e, ky).and_then(|s| s.parse().ok())?;
    Some((x, y))
}

/// 解析 `a:prstGeom` 内的 `a:avLst > a:gd`(§3.j)-> `(name, val)` 对。
/// `fmla` 形如 `"val 25000"`,取末 token 解析;非 `val` 公式跳过(保持保守)。
/// 已消费 `<a:prstGeom>` 起始标签,消费到其结束标签。
fn parse_av_lst<R: std::io::BufRead>(reader: &mut Reader<R>) -> Vec<(String, i64)> {
    let mut adjusts = Vec::new();
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                if let Some(gd) = gd_of(&e) {
                    adjusts.push(gd);
                }
            }
            Ok(Event::Empty(e)) => {
                if let Some(gd) = gd_of(&e) {
                    adjusts.push(gd);
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
    adjusts
}

/// 识别一个 `<a:gd name="adj" fmla="val 50000"/>`;非 gd / 非 val 公式 → `None`。
fn gd_of(e: &BytesStart) -> Option<(String, i64)> {
    if local_name(e.name().as_ref()) != b"gd" {
        return None;
    }
    let name = attr_of(e, b"name")?;
    let fmla = attr_of(e, b"fmla")?;
    let mut it = fmla.split_whitespace();
    if it.next() != Some("val") {
        return None;
    }
    let val: i64 = it.next()?.parse().ok()?;
    Some((name, val))
}

/// 解析 `a:gradFill` 的 stop 颜色(`a:gsLst > a:gs` 内首个颜色元素,按文档顺序)。
/// 已消费 `<a:gradFill>` 起始标签,消费到其结束标签。
fn parse_grad_fill<R: std::io::BufRead>(reader: &mut Reader<R>) -> Vec<ColorSpec> {
    let mut stops = Vec::new();
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"gs" {
                    // parse_color_in 消费到 </a:gs>,不影响 depth。
                    if let Some(spec) = parse_color_in(reader) {
                        stops.push(spec);
                    }
                } else {
                    depth += 1;
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
    stops
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

/// 解析 `p:txBody` -> 段落序列 + 自带列表样式 + `a:bodyPr`(B-6)。
/// 已消费 `<p:txBody>` 起始标签。
fn parse_txbody<R: std::io::BufRead>(reader: &mut Reader<R>) -> TxBodyData {
    let mut body = TxBodyData::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"p" => body.paragraphs.push(parse_paragraph(reader)),
                    b"bodyPr" => body.body = parse_body_pr(reader, &e),
                    b"lstStyle" => {
                        let ls = parse_list_style(reader);
                        if !ls.is_empty() {
                            body.list_style = Some(ls);
                        }
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                // `<a:bodyPr .../>` 常见自闭合(占位符缺省形)。
                if local_name(e.name().as_ref()) == b"bodyPr" {
                    body.body = body_pr_attrs(&e);
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

/// `a:bodyPr` 属性上的文本体属性(锚定 / 内边距 / 换行 / 文字方向,§3.f)。
fn body_pr_attrs(e: &BytesStart) -> BodyProps {
    let emu = |k: &[u8]| attr_of(e, k).and_then(|s| s.parse().ok());
    BodyProps {
        anchor: attr_of(e, b"anchor"),
        anchor_ctr: attr_of(e, b"anchorCtr").map(ooxml_bool),
        l_ins: emu(b"lIns"),
        t_ins: emu(b"tIns"),
        r_ins: emu(b"rIns"),
        b_ins: emu(b"bIns"),
        // `wrap="none"` 显式关;`wrap="square"` 显式开;缺失 → 继承。
        wrap: attr_of(e, b"wrap").map(|v| v != "none"),
        vert: attr_of(e, b"vert"),
        autofit: None,
    }
}

/// 解析 `a:bodyPr`(非自闭合):属性 + 自动适配子元素
/// (`a:normAutofit@fontScale/@lnSpcReduction` / `a:spAutoFit` / `a:noAutofit`)。
/// 已消费起始标签;`start` 是该起始标签。
fn parse_body_pr<R: std::io::BufRead>(reader: &mut Reader<R>, start: &BytesStart) -> BodyProps {
    let mut bp = body_pr_attrs(start);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                autofit_of(&mut bp, &name, &e);
            }
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                autofit_of(&mut bp, &name, &e);
                skip_element(reader, &name);
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    bp
}

/// 识别一个自动适配子元素并填入 `bp.autofit`(非适配元素忽略)。
fn autofit_of(bp: &mut BodyProps, name: &[u8], e: &BytesStart) {
    match name {
        b"normAutofit" => {
            bp.autofit = Some(Autofit::Normal {
                font_scale: attr_of(e, b"fontScale").and_then(|s| s.parse().ok()),
                ln_spc_reduction: attr_of(e, b"lnSpcReduction").and_then(|s| s.parse().ok()),
            });
        }
        b"spAutoFit" => bp.autofit = Some(Autofit::Shape),
        b"noAutofit" => bp.autofit = Some(Autofit::None),
        _ => {}
    }
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
                    b"xfrm" => rect = parse_xfrm(reader, &e).rect.or(rect),
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

/// 解析 `a:tbl` -> `Table`(`a:tblGrid` 列宽 + `a:tr` 行 + `a:tblPr` 样式 id)。
/// 已消费 `<a:tbl>` 起始标签。
fn parse_table<R: std::io::BufRead>(reader: &mut Reader<R>, rect: Option<Rect>) -> Table {
    let mut col_widths = Vec::new();
    let mut rows = Vec::new();
    let mut table_style_id = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"tblGrid" => col_widths = parse_tbl_grid(reader),
                    b"tblPr" => table_style_id = parse_tbl_pr(reader).or(table_style_id),
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
        table_style_id,
    }
}

/// 解析 `a:tblPr` 内的 `a:tableStyleId` 文本(v1 不解析 `tableStyles.xml` 语义,
/// 仅捕获 id 供渲染降级告警,PRD §1)。已消费 `<a:tblPr>` 起始标签。
fn parse_tbl_pr<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<String> {
    let mut id = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"tableStyleId" {
                    let text = read_text(reader);
                    if !text.trim().is_empty() {
                        id = Some(text.trim().to_string());
                    }
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
    id
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
        // tcPr 内边距 / 锚定 / 逐边框线的解析属 B-7 后续;缺省在终态 IR 回填。
        mar_l: None,
        mar_r: None,
        mar_t: None,
        mar_b: None,
        anchor: None,
        borders: CellBorders::default(),
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
                    // tcPr 带子元素(填充 / 逐边框线):属性 + 子元素都解析。
                    b"tcPr" => {
                        let mut t = tcpr_attrs(&e);
                        parse_tcpr_children(reader, &mut t);
                        apply_tcpr(&mut cell, t);
                    }
                    _ => skip_element(reader, &name),
                }
            }
            // 自闭合 tcPr(`<a:tcPr marL=.. anchor=../>`):只有属性,无子元素。
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"tcPr" {
                    apply_tcpr(&mut cell, tcpr_attrs(&e));
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    cell
}

/// `a:tcPr` 的解析中间态:填充 + 内边距(EMU)+ 垂直锚定 + 逐边框线(§3.p/§3.q)。
#[derive(Default)]
struct TcPr {
    fill: Option<ColorSpec>,
    mar_l: Option<Emu>,
    mar_r: Option<Emu>,
    mar_t: Option<Emu>,
    mar_b: Option<Emu>,
    anchor: Option<String>,
    borders: CellBorders,
}

/// 读 `a:tcPr` 的属性:`@marL/@marR/@marT/@marB`(内边距 EMU)、`@anchor`(t/ctr/b)。
fn tcpr_attrs(e: &BytesStart) -> TcPr {
    let emu = |k: &[u8]| attr_of(e, k).and_then(|s| s.parse::<Emu>().ok());
    TcPr {
        mar_l: emu(b"marL"),
        mar_r: emu(b"marR"),
        mar_t: emu(b"marT"),
        mar_b: emu(b"marB"),
        anchor: attr_of(e, b"anchor"),
        ..TcPr::default()
    }
}

/// 读 `a:tcPr` 的子元素:`a:solidFill`(单元格填充)、`a:lnL/lnR/lnT/lnB`(逐边框线,
/// 各是一个 `a:ln`——width/dash/solidFill 走 [`parse_ln`];自闭合仅 width)。
/// 对角线 `lnTlToBr`/`lnBlToTr` v1 忽略。
fn parse_tcpr_children<R: std::io::BufRead>(reader: &mut Reader<R>, t: &mut TcPr) {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"solidFill" => t.fill = parse_solid_fill(reader).or(t.fill.take()),
                    b"lnL" => t.borders.left = parse_ln(reader, &e),
                    b"lnR" => t.borders.right = parse_ln(reader, &e),
                    b"lnT" => t.borders.top = parse_ln(reader, &e),
                    b"lnB" => t.borders.bottom = parse_ln(reader, &e),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                let w = || stroke_if_any(None, ln_width(&e), None);
                match name.as_slice() {
                    b"lnL" => t.borders.left = w(),
                    b"lnR" => t.borders.right = w(),
                    b"lnT" => t.borders.top = w(),
                    b"lnB" => t.borders.bottom = w(),
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
}

/// 把解析出的 `tcPr` 落到单元格:填充/锚定仅在存在时覆盖,内边距与边框直接落
/// (缺省在终态 IR 回填内边距;边框 `None` 边不画)。
fn apply_tcpr(cell: &mut Cell, t: TcPr) {
    if t.fill.is_some() {
        cell.fill = t.fill;
    }
    cell.mar_l = t.mar_l;
    cell.mar_r = t.mar_r;
    cell.mar_t = t.mar_t;
    cell.mar_b = t.mar_b;
    if t.anchor.is_some() {
        cell.anchor = t.anchor;
    }
    cell.borders = t.borders;
}

/// 解析 `p:pic`(图片):`p:spPr`(位置 + 旋转/翻转)+ `p:blipFill`(rel id +
/// `srcRect` 裁剪 + `stretch/fillRect` 拉伸目标,§3.n)+ 占位符标识。
/// 已消费 `<p:pic>` 起始标签。
fn parse_pic<R: std::io::BufRead>(reader: &mut Reader<R>, ctx: &Ctx) -> Option<Shape> {
    let mut rect: Option<Rect> = None;
    let mut xfrm = Xfrm::default();
    let mut blip = BlipFillData::default();
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
                        xfrm = pr.xfrm;
                    }
                    b"blipFill" => blip = parse_blip_fill(reader),
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

    let rel_id = blip.rel_id.unwrap_or_default();
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
        xfrm,
        rel_id,
        media_name,
        image_bytes_len,
        src_rect: blip.src_rect,
        fill_rect: blip.fill_rect,
        placeholder,
    }))
}

/// `p:blipFill` 的解析结果:rel id + 源裁剪 + 拉伸目标。
#[derive(Debug, Clone, Default)]
struct BlipFillData {
    rel_id: Option<String>,
    src_rect: Option<RelRect>,
    fill_rect: Option<RelRect>,
}

/// 解析 `p:blipFill`:`a:blip@r:embed`、`a:srcRect`、`a:stretch > a:fillRect`。
/// 已消费 `<p:blipFill>` 起始标签(深度计数消费到其结束标签)。
fn parse_blip_fill<R: std::io::BufRead>(reader: &mut Reader<R>) -> BlipFillData {
    let mut out = BlipFillData::default();
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                blip_fill_elem(&e, &mut out);
            }
            Ok(Event::Empty(e)) => blip_fill_elem(&e, &mut out),
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
    out
}

/// 识别 `blipFill` 内的一个元素(任意深度):`blip`(rel id)/ `srcRect` / `fillRect`。
fn blip_fill_elem(e: &BytesStart, out: &mut BlipFillData) {
    match local_name(e.name().as_ref()) {
        b"blip" => {
            // `r:embed` 属性。
            for attr in e.attributes().flatten() {
                if local_name(attr.key.as_ref()) == b"embed" {
                    out.rel_id = Some(attr_string(&attr));
                }
            }
        }
        b"srcRect" => out.src_rect = Some(rel_rect_of(e)),
        b"fillRect" => out.fill_rect = Some(rel_rect_of(e)),
        _ => {}
    }
}

/// 从 `a:srcRect` / `a:fillRect` 的 `l`/`t`/`r`/`b` 属性建 [`RelRect`](缺省 0)。
fn rel_rect_of(e: &BytesStart) -> RelRect {
    let g = |k: &[u8]| attr_of(e, k).and_then(|s| s.parse().ok()).unwrap_or(0);
    RelRect {
        l: g(b"l"),
        t: g(b"t"),
        r: g(b"r"),
        b: g(b"b"),
    }
}
