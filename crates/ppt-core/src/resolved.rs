//! 继承链解析后的**终态 IR**(PRD-PDF-EXPORT §4.3,B-9):
//! 每个形状携带物化矩形(占位符几何已回填)、终端 RGB(scheme 引用不再存在)、
//! 已展开的主题字体名、逐属性合并完毕的段落 / run 样式(不再有 `Option` 继承语义)。
//! 由 `ppt_parse::resolve` 产出,`ppt-render` 消费;原始解析模型保持不动。

use crate::color::ResolvedColor;
use crate::geom::{Emu, Rect};
use crate::model::{GraphicPlaceholder, Picture, RunKind, Xfrm};
use crate::style::Spacing;

/// 继承链全无字号时的兜底字号(PowerPoint 默认 18 磅)。
pub const DEFAULT_FONT_SIZE_PT: f32 = 18.0;

/// OOXML `bodyPr` / `tcPr` 缺省左右内边距(91440 EMU = 0.1" = 7.2 pt)。
pub const DEFAULT_INSET_LR_EMU: Emu = 91_440;
/// OOXML `bodyPr` / `tcPr` 缺省上下内边距(45720 EMU = 0.05" = 3.6 pt)。
pub const DEFAULT_INSET_TB_EMU: Emu = 45_720;

/// 一份继承链已解析的演示文稿。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPresentation {
    pub slide_size: (Emu, Emu),
    pub slides: Vec<ResolvedSlide>,
}

/// 一张已解析的幻灯片(形状按 spTree 文档顺序 = 绘制顺序)。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSlide {
    pub index: usize,
    /// 背景(slide → layout → master 链第一个获胜;`bgRef` 已经主题终端化,
    /// B-10);`None` = 不铺背景。
    pub background: Option<ResolvedBackground>,
    pub shapes: Vec<ResolvedShape>,
}

/// 已解析的幻灯片背景(B-10)。
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedBackground {
    /// 纯色(渐变已降级为代表色,渲染侧记 `GradientDegraded`)。
    Color(ResolvedFill),
    /// 图片背景(media 裸名;字节在解析输出的 media 表里)。
    Picture { media_name: String },
}

/// 已解析的形状。
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedShape {
    TextBox(ResolvedTextFrame),
    Auto(ResolvedAutoShape),
    Connector(ResolvedConnector),
    Table(ResolvedTable),
    /// 图片:占位符几何已物化到 `rect`,其余原样。
    Picture(Picture),
    /// 组合:自身变换 + 子坐标空间保留,子形状递归解析(渲染侧按
    /// `(child − chOff)·(ext/chExt) + off` 累积仿射,B-5)。
    Group(ResolvedGroup),
    /// 图表 / SmartArt / OLE 占位(原样透传,渲染侧画占位框)。
    Placeholder(GraphicPlaceholder),
}

/// 已解析的组合:子形状仍在原始子坐标系里,重映射交渲染侧累积。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedGroup {
    /// 组合在父坐标系里的矩形(`a:off`/`a:ext`)。
    pub rect: Option<Rect>,
    /// 子坐标空间(`a:chOff`/`a:chExt`)。
    pub child_rect: Option<Rect>,
    /// 组合自身的旋转/翻转。
    pub xfrm: Xfrm,
    pub children: Vec<ResolvedShape>,
}

/// 已解析的形状填充终态。"整条链无填充 / 显式 `noFill`"由外层 `Option` 表达。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResolvedFill {
    /// 纯色填充。
    Solid(ResolvedColor),
    /// 渐变降级出的代表色(首个 stop;渲染侧记 `GradientDegraded`,PRD §1)。
    Gradient(ResolvedColor),
}

impl ResolvedFill {
    /// 终端颜色(渐变即其代表色)。
    #[must_use]
    pub fn color(self) -> ResolvedColor {
        match self {
            ResolvedFill::Solid(c) | ResolvedFill::Gradient(c) => c,
        }
    }
}

/// 垂直锚定终态(`bodyPr@anchor` / `tcPr@anchor` 合并后)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResolvedAnchor {
    /// 顶部(缺省)。
    #[default]
    Top,
    /// 垂直居中(`ctr`)。
    Middle,
    /// 底部(`b`)。
    Bottom,
}

impl ResolvedAnchor {
    /// 从 OOXML anchor 词位映射(`just`/`dist` 近似 Top,与主流实现一致)。
    #[must_use]
    pub fn from_ooxml(v: Option<&str>) -> Self {
        match v {
            Some("ctr") => ResolvedAnchor::Middle,
            Some("b") => ResolvedAnchor::Bottom,
            _ => ResolvedAnchor::Top,
        }
    }
}

/// `bodyPr` 终态(占位符链逐属性合并完毕、OOXML 缺省已回填,B-6)。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedBodyProps {
    /// 垂直锚定。
    pub anchor: ResolvedAnchor,
    /// 锚定居中(`anchorCtr`;v1 渲染不消费,信息保留)。
    pub anchor_ctr: bool,
    /// 内边距(EMU,缺省 91440 / 45720 / 91440 / 45720)。
    pub l_ins: Emu,
    pub t_ins: Emu,
    pub r_ins: Emu,
    pub b_ins: Emu,
    /// 自动换行(缺省 true)。
    pub wrap: bool,
    /// `normAutofit@fontScale` 折成的分数(1.0 = 100%);无 normAutofit 为 `None`。
    pub font_scale: Option<f32>,
    /// `normAutofit@lnSpcReduction` 折成的分数(0.1 = 减 10%)。
    pub ln_spc_reduction: Option<f32>,
    /// `a:normAutofit` 是否生效(文字缩放适配)。区分「无 autofit」与「有
    /// normAutofit 但未存 fontScale」——后者需消费侧按内容**重算**缩放(B-6)。
    pub autofit_normal: bool,
    /// 是否裁剪到形状矩形(normAutofit 语义;PRD §9 风险 3 的锁定行为)。
    pub clip: bool,
    /// 纵排文字(`vert` ≠ `horz`;渲染水平降级 + 告警,PRD §1)。
    pub vertical: bool,
}

impl Default for ResolvedBodyProps {
    /// OOXML 缺省:顶部锚定、缺省内边距、自动换行、无 autofit、横排。
    fn default() -> Self {
        ResolvedBodyProps {
            anchor: ResolvedAnchor::Top,
            anchor_ctr: false,
            l_ins: DEFAULT_INSET_LR_EMU,
            t_ins: DEFAULT_INSET_TB_EMU,
            r_ins: DEFAULT_INSET_LR_EMU,
            b_ins: DEFAULT_INSET_TB_EMU,
            wrap: true,
            font_scale: None,
            ln_spc_reduction: None,
            autofit_normal: false,
            clip: false,
            vertical: false,
        }
    }
}

/// 已解析的文本框:矩形已按 slide → layout → master 物化。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTextFrame {
    pub rect: Option<Rect>,
    /// 旋转/翻转(文本渲染只用旋转;翻转不镜像文字,PowerPoint 语义)。
    pub xfrm: Xfrm,
    /// `bodyPr` 终态(锚定 / 内边距 / 换行 / autofit,B-6)。
    pub body: ResolvedBodyProps,
    pub paragraphs: Vec<ResolvedParagraph>,
}

/// 已解析的自选图形。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAutoShape {
    pub rect: Option<Rect>,
    pub xfrm: Xfrm,
    pub geometry: Option<String>,
    /// 预设几何调整值(`a:avLst`,原样透传给 TS-6)。
    pub adjusts: Vec<(String, i64)>,
    pub fill: Option<ResolvedFill>,
    pub stroke: Option<ResolvedStroke>,
    pub text: Option<ResolvedTextFrame>,
}

/// 已解析的连接线。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConnector {
    pub rect: Option<Rect>,
    pub xfrm: Xfrm,
    pub geometry: Option<String>,
    pub adjusts: Vec<(String, i64)>,
    pub fill: Option<ResolvedFill>,
    pub stroke: Option<ResolvedStroke>,
}

/// 已解析的描边(颜色终端化;宽度 / 虚线原样)。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStroke {
    pub color: Option<ResolvedColor>,
    pub width_emu: Option<Emu>,
    pub dash: Option<String>,
}

/// 已解析的表格。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTable {
    pub rect: Option<Rect>,
    pub col_widths: Vec<Emu>,
    pub rows: Vec<ResolvedRow>,
    /// `tableStyleId`(v1 不解析样式语义;渲染侧据此记一次降级告警,PRD §1)。
    pub table_style_id: Option<String>,
}

/// 已解析的表格行。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRow {
    pub cells: Vec<ResolvedCell>,
    pub height: Option<Emu>,
}

/// 已解析的单元格逐边框线(颜色终端化;`None` 边不画)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResolvedCellBorders {
    pub left: Option<ResolvedStroke>,
    pub right: Option<ResolvedStroke>,
    pub top: Option<ResolvedStroke>,
    pub bottom: Option<ResolvedStroke>,
}

/// 已解析的单元格(填充终端化;文字走非占位符基链;内边距缺省已回填,B-7)。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCell {
    pub paragraphs: Vec<ResolvedParagraph>,
    pub col_span: u32,
    pub row_span: u32,
    pub fill: Option<ResolvedColor>,
    pub merged: bool,
    /// 内边距(EMU,`tcPr@mar*`;缺省 91440 / 45720)。
    pub mar_l: Emu,
    pub mar_r: Emu,
    pub mar_t: Emu,
    pub mar_b: Emu,
    /// 垂直锚定(`tcPr@anchor`,缺省顶部)。
    pub anchor: ResolvedAnchor,
    /// 逐边框线(§3.q)。
    pub borders: ResolvedCellBorders,
}

/// 已解析的段落:层级样式链合并完毕。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedParagraph {
    pub level: u8,
    pub align: Option<String>,
    pub mar_l: Option<Emu>,
    pub indent: Option<Emu>,
    /// 行距(`a:lnSpc` 合并终态;`None` = 单倍,B-6)。
    pub ln_spc: Option<Spacing>,
    /// 段前距(`a:spcBef`)。
    pub spc_bef: Option<Spacing>,
    /// 段后距(`a:spcAft`)。
    pub spc_aft: Option<Spacing>,
    pub bullet: ResolvedBullet,
    pub runs: Vec<ResolvedRun>,
}

/// 已解析的项目符号(继承链合并后的终态;`None` 含显式 `buNone` 压制)。
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedBullet {
    None,
    Char {
        ch: String,
        font: Option<String>,
        /// 相对正文字号的百分比(`buSzPct`,1.0 = 100%)。
        size_pct: Option<f32>,
    },
    AutoNum {
        scheme: Option<String>,
        start_at: Option<i32>,
        font: Option<String>,
        size_pct: Option<f32>,
    },
}

/// 已解析的 run:样式合并完毕、主题字体名已展开、颜色终端 RGB。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRun {
    pub text: String,
    pub kind: RunKind,
    /// 拉丁字体名(主题引用 `+mj-lt`/`+mn-lt` 已展开);链上全缺为 `None`(交渲染兜底)。
    pub font: Option<String>,
    pub ea_font: Option<String>,
    pub cs_font: Option<String>,
    /// 字号(磅;链上全缺时为 [`DEFAULT_FONT_SIZE_PT`])。
    pub size_pt: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// 终端文字色(链上全缺时黑)。
    pub color: ResolvedColor,
}
