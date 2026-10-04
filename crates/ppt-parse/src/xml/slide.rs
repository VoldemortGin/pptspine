//! 解析幻灯片形部件 XML(slide / slideLayout / slideMaster 共用同一 spTree 结构)
//! -> [`PartData`](形状 + 颜色映射 + master 文本样式)。
//!
//! 走 `p:cSld` > `p:spTree`,识别这些节点:
//! - `p:sp`   —— 文本框 / 自选图形(看有没有 `a:prstGeom`);占位符 `p:ph` / 列表样式
//!   `a:lstStyle` / 形状样式 `p:style` 一并捕获(B-8/B-9 继承链)
//! - `p:graphicFrame` > `a:tbl` —— 表格;非表格内容(图表 / SmartArt / OLE)降级为占位
//!   (图表另记 `c:chart@r:id`,解析完 slide 后经 rels 读图表部件回填数据)
//! - `p:pic`  —— 图片
//! - `p:grpSp` —— 组合(递归)
//! - `p:cxnSp` —— 连接线
//! - `mc:AlternateContent` —— 先试 `mc:Choice`(文档顺序第一个解析出内容的),全空才取 `mc:Fallback`
//!   (形状树层与 `a:p` 段落层同策略,与 docspine 对齐;绝不同时取两支)
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
    GraphicPlaceholder, GroupShape, Hyperlink, Paragraph, Picture, RelRect, Row, RunKind, Shape,
    Stroke, Table, TableFlags, TextFrame, TextRun, Xfrm,
};
use ppt_core::model::{LineEnd, LineEndKind, LineEndSize};
use ppt_core::style::{
    FontRef, PlaceholderRef, RunStyle, ShapeStyle, StyleMatrixRef, TextStyleLevels, TxStyles,
};
use ppt_core::theme::ClrMap;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::text_style::{
    hyperlink_from, level_style_attrs, parse_color_in, parse_level_style, parse_list_style,
    parse_run_props, parse_solid_fill, run_style_attrs,
};
use super::Relationship;
use super::{
    attr_of, attr_string, bool_attr, local_name, ooxml_bool, parse_rels, read_text, skip_element,
};

/// `p:txBody` 的解析结果:段落 + 自带 `a:lstStyle` + `a:bodyPr`。
#[derive(Debug, Clone, Default)]
pub(super) struct TxBodyData {
    pub(super) paragraphs: Vec<Paragraph>,
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
    /// 根元素 `@show="0"`(隐藏页;仅 slide 有意义)。
    pub hidden: bool,
    /// 根元素 `p:sld@showMasterSp` / `p:sldLayout@showMasterSp`;`None` = 缺省(显示)。
    pub show_master_sp: Option<bool>,
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
        depth: std::cell::Cell::new(0),
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
                    b"sld" => {
                        out.hidden = attr_of(&e, b"show").is_some_and(|v| !ooxml_bool(v));
                        out.show_master_sp = attr_of(&e, b"showMasterSp").map(ooxml_bool);
                    }
                    b"sldLayout" => {
                        out.show_master_sp = attr_of(&e, b"showMasterSp").map(ooxml_bool);
                    }
                    // 其余容器(cSld / sldMaster …)继续下钻。
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

/// 形状容器(`p:grpSp` / `mc:AlternateContent`)的最大嵌套深度。超过的子树整体跳过
/// (与"未知元素跳过"同一降级口径),防恶意深嵌套把递归下降解析压爆栈;下游
/// resolve / render / export 对组合的递归也因此有界。
const MAX_NEST_DEPTH: u32 = 64;

/// 解析期的上下文(`depth` 为当前形状容器嵌套深度)。
struct Ctx<'a> {
    rels: &'a BTreeMap<String, Relationship>,
    media_index: &'a BTreeMap<String, usize>,
    depth: std::cell::Cell<u32>,
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
        b"grpSp" | b"AlternateContent" => {
            let depth = ctx.depth.get();
            if depth >= MAX_NEST_DEPTH {
                // 嵌套过深:整棵子树跳过(skip_element 是迭代的,不吃栈)。
                skip_element(reader, name);
                return true;
            }
            ctx.depth.set(depth + 1);
            if name == b"grpSp" {
                out.push(Shape::Group(parse_grp_sp(reader, ctx)));
            } else {
                parse_alternate_content(reader, ctx, out);
            }
            ctx.depth.set(depth);
        }
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

/// 解析 `mc:AlternateContent`(Markup Compatibility,ECMA-376 Part 3),与 docspine 同策略:
/// 按文档顺序把每个 `mc:Choice` 交给形状解析试一遍,第一个**产出非空形状**的被选中;
/// Choice 里常是本仓不认识的新版元素(`p14:` / `a14:` …),解析为空是预期的,此时回落
/// `mc:Fallback`。流式 reader 无法回退,故各分支顺序解析、选中后其余丢弃——Choice 与
/// Fallback 内容绝不同时输出。已消费起始标签。
fn parse_alternate_content<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    ctx: &Ctx,
    out: &mut Vec<Shape>,
) {
    let mut chosen: Option<Vec<Shape>> = None;
    let mut fallback: Option<Vec<Shape>> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"Choice" if chosen.is_none() => {
                        let mut shapes = Vec::new();
                        parse_shapes_into(reader, ctx, &mut shapes);
                        if !shapes.is_empty() {
                            chosen = Some(shapes);
                        }
                    }
                    b"Fallback" if chosen.is_none() && fallback.is_none() => {
                        let mut shapes = Vec::new();
                        parse_shapes_into(reader, ctx, &mut shapes);
                        fallback = Some(shapes);
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
    out.extend(chosen.or(fallback).unwrap_or_default());
}

/// 解析一个 `p:sp`(文本框或自选图形)。已消费 `<p:sp>` 起始标签。
fn parse_sp<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<Shape> {
    let mut pr = SpPr::default();
    let mut placeholder: Option<PlaceholderRef> = None;
    let mut hyperlink: Option<Hyperlink> = None;
    let mut style: Option<ShapeStyle> = None;
    let mut body = TxBodyData::default();
    let mut has_txbody = false;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"nvSpPr" => {
                        let nv = parse_nv(reader);
                        placeholder = nv.ph.or(placeholder);
                        hyperlink = nv.hyperlink.or(hyperlink);
                    }
                    b"spPr" => {
                        let got = parse_sppr(reader);
                        pr.rect = got.rect.or(pr.rect);
                        pr.xfrm = got.xfrm;
                        pr.geometry = got.geometry.or(pr.geometry);
                        pr.adjusts = got.adjusts;
                        pr.fill = got.fill.or(pr.fill);
                        pr.stroke = got.stroke.or(pr.stroke);
                        pr.custom_geometry |= got.custom_geometry;
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
        hyperlink: hyperlink.clone(),
    };
    // 有预设几何 / 实际填充 / 描边 => 当作自选图形;否则当作纯文本框
    // (孤立的显式 `noFill` 不改变分类——渲染结果与纯文本框一致)。
    // `a:custGeom` 且仅经 `p:style` 的 fillRef / lnRef 取色,同样是会着色的图形:
    // 归为自选图形(渲染按包围盒降级 + 告警),而不是静默丢成不画的纯文本框。
    let has_paint = pr.fill.as_ref().is_some_and(|f| !matches!(f, Fill::None));
    let style_paint = style.as_ref().is_some_and(|st| {
        let live = |r: &Option<StyleMatrixRef>| r.as_ref().is_some_and(|r| r.idx >= 1);
        live(&st.fill_ref) || live(&st.ln_ref)
    });
    if pr.geometry.is_some()
        || has_paint
        // 显式无线(`no_fill`)本身不构成描边:保持旧分类(无几何 / 填充时仍是文本框)。
        || pr.stroke.as_ref().is_some_and(|s| !s.no_fill)
        || (pr.custom_geometry && style_paint)
    {
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
            custom_geometry: pr.custom_geometry,
            hyperlink,
        }))
    } else {
        Some(Shape::TextBox(text_frame))
    }
}

/// 非可视属性容器(`p:nvSpPr` / `p:nvPicPr`)里捕获的信息。
#[derive(Default)]
struct NvProps {
    /// `p:nvPr > p:ph`(占位符标识)。
    ph: Option<PlaceholderRef>,
    /// `p:cNvPr@name`。
    name: Option<String>,
    /// `p:cNvPr@descr`(替代文本)。
    descr: Option<String>,
    /// `p:cNvPr@title`。
    title: Option<String>,
    /// `p:cNvPr > a:hlinkClick`(形状级超链接)。
    hyperlink: Option<Hyperlink>,
}

impl NvProps {
    fn take(&mut self, e: &BytesStart) {
        match local_name(e.name().as_ref()) {
            b"ph" if self.ph.is_none() => self.ph = Some(ph_from(e)),
            b"cNvPr" => {
                let get = |k: &[u8]| attr_of(e, k).filter(|v| !v.is_empty());
                self.name = get(b"name");
                self.descr = get(b"descr");
                self.title = get(b"title");
            }
            b"hlinkClick" if self.hyperlink.is_none() => self.hyperlink = Some(hyperlink_from(e)),
            _ => {}
        }
    }
}

/// 解析 `p:nvSpPr` / `p:nvPicPr` 等非可视属性容器:`p:ph`(占位符标识)+
/// `p:cNvPr`(name / descr / title / `a:hlinkClick`)。已消费容器起始标签,消费到其结束标签。
fn parse_nv<R: std::io::BufRead>(reader: &mut Reader<R>) -> NvProps {
    let mut nv = NvProps::default();
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                nv.take(&e);
            }
            Ok(Event::Empty(e)) => nv.take(&e),
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
    nv
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
                        pr.custom_geometry |= got.custom_geometry;
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
        custom_geometry: pr.custom_geometry,
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
    /// 出现了 `a:custGeom`(自定义几何;v1 不求值路径公式)。
    custom_geometry: bool,
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
                    b"custGeom" => {
                        pr.custom_geometry = true;
                        skip_element(reader, &name);
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
                    b"custGeom" => pr.custom_geometry = true,
                    b"noFill" => pr.fill = Some(Fill::None),
                    b"ln" => pr.stroke = bare_ln(&e).or(pr.stroke),
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
    // 深度计数:`a:off` 等叶子既可自闭合也可写成展开形式(`<a:off ..></a:off>`),展开形式的
    // 结束标签不能被当成 `a:xfrm` 自己的结束,否则会提前返回、让父级 spPr 的解析位置失步。
    let mut take = |e: &BytesStart| match local_name(e.name().as_ref()) {
        b"off" => off = xy_of(e, b"x", b"y"),
        b"ext" => ext = xy_of(e, b"cx", b"cy"),
        b"chOff" => ch_off = xy_of(e, b"x", b"y"),
        b"chExt" => ch_ext = xy_of(e, b"cx", b"cy"),
        _ => {}
    };
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                take(&e);
            }
            Ok(Event::Empty(e)) => take(&e),
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
/// 虚线预设 + `a:headEnd` / `a:tailEnd` 线端装饰。已消费 `<a:ln>` 起始标签;`start`
/// 是该起始标签(读取 `w`)。颜色 / 线宽 / 虚线 / 有效线端全缺时返回 `None`(与旧行为
/// 一致:空 `a:ln` 不产生描边)。`a:ln > a:noFill`(不可见线)返回 `Stroke { no_fill: true, .. }`
/// (其余字段清空)——显式无线必须能压制 `lnRef` 主题线,否则只剩线宽会被画成缺省黑线;
/// 其下的线端装饰同样丢弃。
pub(crate) fn parse_ln<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    start: &BytesStart,
) -> Option<Stroke> {
    match parse_ln_no_fill(reader, start) {
        (_, true) => Some(Stroke {
            no_fill: true,
            ..Stroke::default()
        }),
        (stroke, false) => stroke,
    }
}

/// 同 [`parse_ln`],另返回是否含 `a:noFill`(表格样式边框据此区分"显式无线"与"未指定")。
pub(crate) fn parse_ln_no_fill<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    start: &BytesStart,
) -> (Option<Stroke>, bool) {
    let mut stroke = Stroke {
        width_emu: ln_width(start),
        ..Stroke::default()
    };
    let mut no_fill = false;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"solidFill" => stroke.color = parse_solid_fill(reader).or(stroke.color),
                    b"prstDash" => {
                        stroke.dash = attr_of(&e, b"val").or(stroke.dash);
                        skip_element(reader, &name);
                    }
                    b"headEnd" | b"tailEnd" | b"noFill" => {
                        ln_child_empty(&mut stroke, &mut no_fill, &name, &e);
                        skip_element(reader, &name);
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                // `<a:prstDash val="dash"/>` / `<a:tailEnd type="triangle"/>` 通常是自闭合。
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"prstDash" {
                    stroke.dash = attr_of(&e, b"val").or(stroke.dash);
                } else {
                    ln_child_empty(&mut stroke, &mut no_fill, &name, &e);
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    if no_fill {
        stroke.head_end = None;
        stroke.tail_end = None;
    }
    (stroke_if_any(stroke), no_fill)
}

/// `a:ln` 的线端 / `noFill` 子元素(属性即全部信息)。
fn ln_child_empty(stroke: &mut Stroke, no_fill: &mut bool, name: &[u8], e: &BytesStart) {
    match name {
        b"headEnd" => stroke.head_end = Some(line_end_of(e)),
        b"tailEnd" => stroke.tail_end = Some(line_end_of(e)),
        b"noFill" => *no_fill = true,
        _ => {}
    }
}

/// `a:headEnd` / `a:tailEnd` 的 `@type` / `@w` / `@len`(缺失取 ECMA-376 缺省
/// `none` / `med` / `med`;规范外尺寸按 `med`,规范外种类原样进 [`LineEndKind::Other`])。
fn line_end_of(e: &BytesStart) -> LineEnd {
    let size = |key: &[u8]| match attr_of(e, key).as_deref() {
        Some("sm") => LineEndSize::Small,
        Some("lg") => LineEndSize::Large,
        _ => LineEndSize::Medium,
    };
    let kind = match attr_of(e, b"type").as_deref() {
        None | Some("none") => LineEndKind::None,
        Some("triangle") => LineEndKind::Triangle,
        Some("stealth") => LineEndKind::Stealth,
        Some("diamond") => LineEndKind::Diamond,
        Some("oval") => LineEndKind::Oval,
        Some("arrow") => LineEndKind::Arrow,
        Some(other) => LineEndKind::Other(other.to_string()),
    };
    LineEnd {
        kind,
        width: size(b"w"),
        length: size(b"len"),
    }
}

/// `a:ln@w`(EMU 线宽)。
fn ln_width(e: &BytesStart) -> Option<Emu> {
    attr_of(e, b"w").and_then(|s| s.parse().ok())
}

/// 自闭合 `<a:ln w="…"/>`(无子元素,只有线宽)。
pub(crate) fn bare_ln(e: &BytesStart) -> Option<Stroke> {
    stroke_if_any(Stroke {
        width_emu: ln_width(e),
        ..Stroke::default()
    })
}

/// 颜色 / 线宽 / 虚线 / 有效线端(种类非 `none`)至少有一项时保留 [`Stroke`],
/// 否则 `None`——显式 `type="none"` 的线端不单独构成描边(渲染零变化)。
fn stroke_if_any(stroke: Stroke) -> Option<Stroke> {
    let has_end = |e: &Option<LineEnd>| e.as_ref().is_some_and(|e| e.kind != LineEndKind::None);
    if stroke.color.is_none()
        && stroke.width_emu.is_none()
        && stroke.dash.is_none()
        && !has_end(&stroke.head_end)
        && !has_end(&stroke.tail_end)
    {
        return None;
    }
    Some(stroke)
}

/// 解析 `p:txBody` -> 段落序列 + 自带列表样式 + `a:bodyPr`(B-6)。
/// 已消费 `<p:txBody>` 起始标签。
pub(super) fn parse_txbody<R: std::io::BufRead>(reader: &mut Reader<R>) -> TxBodyData {
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
            Ok(Event::Empty(e)) => match local_name(e.name().as_ref()) {
                // `<a:bodyPr .../>` 常见自闭合(占位符缺省形)。
                b"bodyPr" => body.body = body_pr_attrs(&e),
                // 自闭合空段落 `<a:p/>` 与 `<a:p></a:p>` 等价:保留一个空段落(占一行高度)。
                b"p" => body.paragraphs.push(Paragraph::default()),
                _ => {}
            },
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
/// `a:fld`(字段,如页码/日期)、`a14:m`(公式)、`mc:AlternateContent`(先 Choice 后 Fallback)。
/// 已消费 `<a:p>` 起始标签。
fn parse_paragraph<R: std::io::BufRead>(reader: &mut Reader<R>) -> Paragraph {
    let mut para = Paragraph::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"pPr" {
                    para.level = ppr_level(&e);
                    para.props = parse_level_style(reader, &e);
                    para.align = para.props.align.clone();
                } else if !run_elem_start(&name, &e, reader, 0, &mut para.runs) {
                    skip_element(reader, &name);
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"pPr" {
                    para.level = ppr_level(&e);
                    para.props = level_style_attrs(&e);
                    para.align = para.props.align.clone();
                } else {
                    run_elem_empty(&name, &e, &mut para.runs);
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

/// 段落内 run 类元素(`a:r` / `a:br` / `a:fld` / `a14:m` / `mc:AlternateContent`)的起始标签
/// 分发,把产出的 run 追加到 `runs`。非 run 类元素返回 `false`(调用方自行跳过)。
/// `alt_depth` 是已嵌套的 AlternateContent 层数,超过 [`MAX_NEST_DEPTH`] 整棵跳过。
fn run_elem_start<R: std::io::BufRead>(
    name: &[u8],
    e: &BytesStart,
    reader: &mut Reader<R>,
    alt_depth: u32,
    runs: &mut Vec<TextRun>,
) -> bool {
    match name {
        b"r" => runs.push(parse_run_like(reader, RunKind::Text)),
        b"br" => {
            // `<a:br>` 可带 `a:rPr` 子元素,整体消费掉;换行本身无文字样式语义。
            skip_element(reader, name);
            runs.push(break_run());
        }
        b"fld" => {
            let field_type = attr_of(e, b"type");
            runs.push(parse_run_like(reader, RunKind::Field { field_type }));
        }
        b"m" => runs.extend(parse_math(reader)),
        b"AlternateContent" => {
            if alt_depth >= MAX_NEST_DEPTH {
                skip_element(reader, name);
            } else {
                runs.extend(parse_alt_runs(reader, alt_depth + 1));
            }
        }
        _ => return false,
    }
    true
}

/// 段落内 run 类元素的自闭合形式(`<a:br/>` / `<a:fld/>`)。
fn run_elem_empty(name: &[u8], e: &BytesStart, runs: &mut Vec<TextRun>) {
    match name {
        b"br" => runs.push(break_run()),
        b"fld" => {
            // 自闭合字段:无缓存文本,仍保留字段类型(信息无损)。
            runs.push(TextRun {
                kind: RunKind::Field {
                    field_type: attr_of(e, b"type"),
                },
                ..TextRun::default()
            });
        }
        _ => {}
    }
}

/// 段落内 `mc:AlternateContent` -> 选中分支的 run 序列;策略同形状层
/// ([`parse_alternate_content`]):第一个产出带文字 run 的 Choice,否则 Fallback。已消费起始标签。
fn parse_alt_runs<R: std::io::BufRead>(reader: &mut Reader<R>, alt_depth: u32) -> Vec<TextRun> {
    let mut chosen: Option<Vec<TextRun>> = None;
    let mut fallback: Option<Vec<TextRun>> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"Choice" if chosen.is_none() => {
                        let runs = parse_run_container(reader, alt_depth);
                        if runs.iter().any(|r| !r.text.is_empty()) {
                            chosen = Some(runs);
                        }
                    }
                    b"Fallback" if chosen.is_none() && fallback.is_none() => {
                        fallback = Some(parse_run_container(reader, alt_depth));
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
    chosen.or(fallback).unwrap_or_default()
}

/// 解析 `mc:Choice` / `mc:Fallback` 在段落内的 run 序列,直到其结束标签。
fn parse_run_container<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    alt_depth: u32,
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if !run_elem_start(&name, &e, reader, alt_depth, &mut runs) {
                    skip_element(reader, &name);
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                run_elem_empty(&name, &e, &mut runs);
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    runs
}

// ============================================================ 公式 (a14:m / m:oMath)
//
// 线性化规则与兄弟仓 docspine(`doc-parse/src/xml/document.rs` 的 `parse_math`)保持一致,
// 两仓各自独立实现、无跨仓依赖:`m:f` -> `分子/分母`、`m:sSup` -> `x^2`、`m:sSub` -> `x_i`、
// `m:rad` -> `sqrt(x)`(带次数 `root(3,x)`);某一项含多于一个文字片段或本身是复合结构时
// 加括号;其它结构按文档顺序纯拼接 `m:t`。

/// 公式里需要线性化的结构。
#[derive(Clone, Copy, PartialEq)]
enum MathStruct {
    Frac,
    Sup,
    Sub,
    Rad,
}

/// 结构内的槽位元素。
#[derive(Clone, Copy, PartialEq)]
enum MathSlot {
    Num,
    Den,
    Base,
    Sup,
    Sub,
    Deg,
}

/// 公式遍历栈上的一帧:容器 / 结构 / 槽位各自攒一份文字。其余嵌套元素不开帧,只在帧内记
/// `other_depth`,文字直接并入当前帧(纯拼接且不随深度反复复制)。
struct MathFrame {
    /// 本帧是哪种结构(`None` = 容器或槽位)。
    kind: Option<MathStruct>,
    /// 本帧若是槽位,它是哪个槽。
    slot: Option<MathSlot>,
    text: String,
    /// 本帧内 `m:t` 文字片段数(复合子项记 2),决定线性化时是否加括号。
    frags: usize,
    /// 结构帧:已收齐的槽位 `(槽, 文字, 片段数)`。
    slots: Vec<(MathSlot, String, usize)>,
    /// 帧内未开帧的嵌套元素深度。
    other_depth: usize,
}

impl MathFrame {
    fn new(kind: Option<MathStruct>, slot: Option<MathSlot>) -> Self {
        MathFrame {
            kind,
            slot,
            text: String::new(),
            frags: 0,
            slots: Vec::new(),
            other_depth: 0,
        }
    }
}

fn math_struct_of(name: &[u8]) -> Option<MathStruct> {
    match name {
        b"f" => Some(MathStruct::Frac),
        b"sSup" => Some(MathStruct::Sup),
        b"sSub" => Some(MathStruct::Sub),
        b"rad" => Some(MathStruct::Rad),
        _ => None,
    }
}

/// 槽位元素本地名 -> 在给定结构里的槽位(不属于该结构的名字不算槽位)。
fn math_slot_of(kind: MathStruct, name: &[u8]) -> Option<MathSlot> {
    match (kind, name) {
        (MathStruct::Frac, b"num") => Some(MathSlot::Num),
        (MathStruct::Frac, b"den") => Some(MathSlot::Den),
        (MathStruct::Sup, b"e") | (MathStruct::Sub, b"e") | (MathStruct::Rad, b"e") => {
            Some(MathSlot::Base)
        }
        (MathStruct::Sup, b"sup") => Some(MathSlot::Sup),
        (MathStruct::Sub, b"sub") => Some(MathSlot::Sub),
        (MathStruct::Rad, b"deg") => Some(MathSlot::Deg),
        _ => None,
    }
}

/// 多于一个文字片段(或复合子项)时用括号包起来。
fn math_wrap(text: &str, frags: usize) -> String {
    if frags > 1 {
        format!("({text})")
    } else {
        text.to_string()
    }
}

/// 把一个结构帧收拢成线性记法文本;所有槽位都为空时返回空串。
fn math_linearize(kind: MathStruct, slots: &[(MathSlot, String, usize)]) -> String {
    if slots.iter().all(|(_, t, _)| t.is_empty()) {
        return String::new();
    }
    let slot = |want: MathSlot| {
        slots
            .iter()
            .find(|(s, _, _)| *s == want)
            .map(|(_, t, n)| (t.as_str(), *n))
            .unwrap_or(("", 0))
    };
    let wrapped = |want: MathSlot| {
        let (t, n) = slot(want);
        math_wrap(t, n)
    };
    match kind {
        MathStruct::Frac => format!("{}/{}", wrapped(MathSlot::Num), wrapped(MathSlot::Den)),
        MathStruct::Sup => format!("{}^{}", wrapped(MathSlot::Base), wrapped(MathSlot::Sup)),
        MathStruct::Sub => format!("{}_{}", wrapped(MathSlot::Base), wrapped(MathSlot::Sub)),
        MathStruct::Rad => {
            let (base, _) = slot(MathSlot::Base);
            match slot(MathSlot::Deg) {
                ("", _) => format!("sqrt({base})"),
                (deg, _) => format!("root({deg},{base})"),
            }
        }
    }
}

/// 把结束的帧并入父帧:槽位 -> 父结构的槽表;结构 -> 线性化文字(作复合项,记 2 片段)。
fn math_merge(parent: &mut MathFrame, done: MathFrame) {
    if let (Some(slot), true) = (done.slot, parent.kind.is_some()) {
        parent.slots.push((slot, done.text, done.frags));
    } else if let Some(kind) = done.kind {
        let out = math_linearize(kind, &done.slots);
        if !out.is_empty() {
            parent.text.push_str(&out);
            parent.frags += 2;
        }
    }
}

/// 解析 `a14:m`(内含 `m:oMathPara` / `m:oMath`)-> 一个 [`RunKind::Math`] run;抽不出文字
/// 返回 `None`。已消费起始标签。**迭代**遍历(显式栈,不递归),结构帧深度受
/// [`MAX_NEST_DEPTH`] 约束(更深的结构退化为纯拼接),深嵌套不会栈溢出;畸形(Eof 时帧未
/// 闭合)自内向外并入父帧,文字不丢。`m:oMathPara` 内多个 `m:oMath` 以空格分隔。
fn parse_math<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<TextRun> {
    let mut stack = vec![MathFrame::new(None, None)];
    let mut struct_depth = 0u32;
    // 根帧上透明展开的 `m:oMathPara` / `m:oMath` 外壳层数(`a14:m` 内总有这层包装)。
    let mut wrappers = 0usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                let name = name.as_slice();
                // 栈永不为空(根帧只在容器结束时才处理),下面的 last_mut 都成立。
                let at_root = stack.len() == 1;
                let Some(top) = stack.last_mut() else { break };
                match name {
                    b"oMathPara" | b"oMath" if at_root && top.other_depth == 0 => {
                        if name == b"oMath" && !top.text.is_empty() {
                            top.text.push(' ');
                        }
                        wrappers += 1;
                    }
                    b"t" => {
                        let t = read_text(reader);
                        if !t.is_empty() {
                            top.text.push_str(&t);
                            top.frags += 1;
                        }
                    }
                    _ if top.other_depth == 0 && top.kind.is_some() => {
                        // 结构帧的直接子元素:认得的槽位开新帧,其余当普通嵌套。
                        match top.kind.and_then(|k| math_slot_of(k, name)) {
                            Some(slot) => stack.push(MathFrame::new(None, Some(slot))),
                            None => top.other_depth += 1,
                        }
                    }
                    _ if top.other_depth == 0 && struct_depth < MAX_NEST_DEPTH => {
                        match math_struct_of(name) {
                            Some(kind) => {
                                struct_depth += 1;
                                stack.push(MathFrame::new(Some(kind), None));
                            }
                            None => top.other_depth += 1,
                        }
                    }
                    _ => top.other_depth += 1,
                }
            }
            Ok(Event::End(_)) => {
                let Some(top) = stack.last_mut() else { break };
                if top.other_depth > 0 {
                    top.other_depth -= 1;
                } else if stack.len() == 1 {
                    if wrappers == 0 {
                        break; // `a14:m` 自身结束。
                    }
                    wrappers -= 1;
                } else if let Some(done) = stack.pop() {
                    if done.kind.is_some() {
                        struct_depth -= 1;
                    }
                    if let Some(parent) = stack.last_mut() {
                        math_merge(parent, done);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    while stack.len() > 1 {
        let Some(done) = stack.pop() else { break };
        if let Some(parent) = stack.last_mut() {
            math_merge(parent, done);
        }
    }
    let text = stack.pop().map(|f| f.text).unwrap_or_default();
    if text.is_empty() {
        return None;
    }
    Some(TextRun {
        text,
        kind: RunKind::Math,
        ..TextRun::default()
    })
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
    let mut hyperlink = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"rPr" => (rs, hyperlink) = parse_run_props(reader, &e),
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
        char_spacing_pt: rs.char_spacing_pt,
        baseline: rs.baseline,
        cap: rs.cap,
        hyperlink,
    }
}

/// 解析 `p:graphicFrame`:其内 `a:graphic` > `a:graphicData` > `a:tbl` -> 表格;
/// 非表格内容(图表 / SmartArt / OLE 等)降级为 [`Shape::Placeholder`],至少保住外框
/// 矩形与 `graphicData@uri`(渲染侧据此画占位框 + 告警)。已消费起始标签。
fn parse_graphic_frame<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<Shape> {
    let mut rect: Option<Rect> = None;
    let mut table: Option<Table> = None;
    let mut uri: Option<String> = None;
    let mut chart_rel_id: Option<String> = None;
    let mut diagram_rel_id: Option<String> = None;
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
                    b"chart" => {
                        chart_rel_id = attr_of(&e, b"id").or(chart_rel_id);
                        skip_element(reader, &name);
                    }
                    b"relIds" => {
                        diagram_rel_id = attr_of(&e, b"dm").or(diagram_rel_id);
                        skip_element(reader, &name);
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => match local_name(e.name().as_ref()) {
                b"graphicData" => uri = attr_of(&e, b"uri").or(uri),
                b"chart" => chart_rel_id = attr_of(&e, b"id").or(chart_rel_id),
                b"relIds" => diagram_rel_id = attr_of(&e, b"dm").or(diagram_rel_id),
                _ => {}
            },
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
        None => Some(Shape::Placeholder(GraphicPlaceholder {
            rect,
            kind: uri,
            chart_rel_id,
            chart: None,
            diagram_rel_id,
            diagram_text: Vec::new(),
        })),
    }
}

/// 解析 `a:tbl` -> `Table`(`a:tblGrid` 列宽 + `a:tr` 行 + `a:tblPr` 样式 id 与开关属性)。
/// 已消费 `<a:tbl>` 起始标签。
fn parse_table<R: std::io::BufRead>(reader: &mut Reader<R>, rect: Option<Rect>) -> Table {
    let mut col_widths = Vec::new();
    let mut rows = Vec::new();
    let mut table_style_id = None;
    let mut flags = TableFlags::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"tblGrid" => col_widths = parse_tbl_grid(reader),
                    b"tblPr" => {
                        flags = tbl_pr_flags(&e);
                        table_style_id = parse_tbl_pr(reader).or(table_style_id);
                    }
                    b"tr" => {
                        let height = attr_of(&e, b"h").and_then(|s| s.parse().ok());
                        let cells = parse_table_row(reader);
                        rows.push(Row { cells, height });
                    }
                    _ => skip_element(reader, &name),
                }
            }
            // 自闭合 `<a:tblPr firstRow="1" bandRow="1"/>`:只有开关属性。
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"tblPr" {
                    flags = tbl_pr_flags(&e);
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
        flags,
    }
}

/// `a:tblPr` 的开关属性(缺失为关)。
fn tbl_pr_flags(e: &BytesStart) -> TableFlags {
    TableFlags {
        first_row: bool_attr(e, b"firstRow"),
        last_row: bool_attr(e, b"lastRow"),
        first_col: bool_attr(e, b"firstCol"),
        last_col: bool_attr(e, b"lastCol"),
        band_row: bool_attr(e, b"bandRow"),
        band_col: bool_attr(e, b"bandCol"),
    }
}

/// 解析 `a:tblPr` 内的 `a:tableStyleId` 文本(指向 `tableStyles.xml` 的样式,在
/// 继承链解析时合进单元格)。已消费 `<a:tblPr>` 起始标签。
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
        no_fill: false,
        merged,
        // 无 `a:tcPr` 时的缺省占位:内边距 / 锚定 / 逐边框线均置 None(表示无覆盖),
        // 缺省在终态 IR 回填。解析本身已实现(tcpr_attrs / parse_tcpr_children);
        // 有 `a:tcPr` 时由 parse_table_cell 的 apply_tcpr 覆盖这些 None。
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
    no_fill: bool,
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

/// 读 `a:tcPr` 的子元素:`a:solidFill`(单元格填充)、`a:noFill`(显式无填充)、
/// `a:lnL/lnR/lnT/lnB`(逐边框线,各是一个 `a:ln`——width/dash/solidFill 走
/// [`parse_ln`];自闭合仅 width)。
/// 对角线 `lnTlToBr`/`lnBlToTr` v1 忽略。
fn parse_tcpr_children<R: std::io::BufRead>(reader: &mut Reader<R>, t: &mut TcPr) {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"solidFill" => t.fill = parse_solid_fill(reader).or(t.fill.take()),
                    b"lnL" => {
                        (t.borders.left, t.borders.no_left) = cell_edge(reader, &e);
                    }
                    b"lnR" => {
                        (t.borders.right, t.borders.no_right) = cell_edge(reader, &e);
                    }
                    b"lnT" => (t.borders.top, t.borders.no_top) = cell_edge(reader, &e),
                    b"lnB" => {
                        (t.borders.bottom, t.borders.no_bottom) = cell_edge(reader, &e);
                    }
                    b"noFill" => {
                        t.no_fill = true;
                        skip_element(reader, &name);
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                let w = || bare_ln(&e);
                match name.as_slice() {
                    b"lnL" => t.borders.left = w(),
                    b"lnR" => t.borders.right = w(),
                    b"lnT" => t.borders.top = w(),
                    b"lnB" => t.borders.bottom = w(),
                    b"noFill" => t.no_fill = true,
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

/// 一条单元格边 `a:lnL` 等(非自闭合):含 `a:noFill` → `(None, true)` 显式无线
/// (否则只剩线宽会被画成缺省黑线);其余同 [`parse_ln`]。
fn cell_edge<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    start: &BytesStart,
) -> (Option<Stroke>, bool) {
    match parse_ln_no_fill(reader, start) {
        (_, true) => (None, true),
        (stroke, false) => (stroke, false),
    }
}

/// 把解析出的 `tcPr` 落到单元格:填充/锚定仅在存在时覆盖,内边距与边框直接落
/// (缺省在终态 IR 回填内边距;边框 `None` 边不画)。
fn apply_tcpr(cell: &mut Cell, t: TcPr) {
    if t.fill.is_some() {
        cell.fill = t.fill;
    }
    cell.no_fill |= t.no_fill;
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
    let mut nv = NvProps::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"nvPicPr" => nv = parse_nv(reader),
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
        placeholder: nv.ph,
        name: nv.name,
        alt_text: nv.descr,
        title: nv.title,
        hyperlink: nv.hyperlink,
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
