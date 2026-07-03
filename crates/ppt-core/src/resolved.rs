//! 继承链解析后的**终态 IR**(PRD-PDF-EXPORT §4.3,B-9):
//! 每个形状携带物化矩形(占位符几何已回填)、终端 RGB(scheme 引用不再存在)、
//! 已展开的主题字体名、逐属性合并完毕的段落 / run 样式(不再有 `Option` 继承语义)。
//! 由 `ppt_parse::resolve` 产出,`ppt-render` 消费;原始解析模型保持不动。

use crate::color::ResolvedColor;
use crate::geom::{Emu, Rect};
use crate::model::{GraphicPlaceholder, Picture, RunKind, Xfrm};

/// 继承链全无字号时的兜底字号(PowerPoint 默认 18 磅)。
pub const DEFAULT_FONT_SIZE_PT: f32 = 18.0;

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
    pub shapes: Vec<ResolvedShape>,
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

/// 已解析的文本框:矩形已按 slide → layout → master 物化。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTextFrame {
    pub rect: Option<Rect>,
    /// 旋转/翻转(文本渲染只用旋转;翻转不镜像文字,PowerPoint 语义)。
    pub xfrm: Xfrm,
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
}

/// 已解析的表格行。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRow {
    pub cells: Vec<ResolvedCell>,
    pub height: Option<Emu>,
}

/// 已解析的单元格(填充终端化;文字走非占位符基链)。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCell {
    pub paragraphs: Vec<ResolvedParagraph>,
    pub col_span: u32,
    pub row_span: u32,
    pub fill: Option<ResolvedColor>,
    pub merged: bool,
}

/// 已解析的段落:层级样式链合并完毕。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedParagraph {
    pub level: u8,
    pub align: Option<String>,
    pub mar_l: Option<Emu>,
    pub indent: Option<Emu>,
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
