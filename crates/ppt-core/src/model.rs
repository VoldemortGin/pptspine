//! pptx 结构化解析的结果模型。
//!
//! 目标是**信息无损**:把 OOXML 里的幻灯片 / 文本 / 表格 / 图片 / 自选图形原样搬进
//! 这些朴素的 `struct` / `enum`。本轮不要求 serde,只派生 `Debug`/`Clone`/`PartialEq`。

use crate::geom::{Emu, Rect};

/// 一份解析好的演示文稿。
#[derive(Debug, Clone, PartialEq)]
pub struct Presentation {
    /// 按 `presentation.xml` 中 `p:sldId` 顺序排列的幻灯片。
    pub slides: Vec<Slide>,
    /// 幻灯片画布尺寸 `(cx, cy)`(EMU,来自 `p:sldSz`)。
    pub slide_size: (Emu, Emu),
}

/// 单张幻灯片。
#[derive(Debug, Clone, PartialEq)]
pub struct Slide {
    /// 零基序号。
    pub index: usize,
    /// 形状树(`p:spTree`)解析出的形状,按文档顺序。
    pub shapes: Vec<Shape>,
    /// 关联的版式名(best-effort)。
    pub layout_name: Option<String>,
    /// 关联的母版名(best-effort)。
    pub master_name: Option<String>,
    /// 演讲者备注文本(`ppt/notesSlides/notesSlideN.xml` 的 body 占位符);无备注为 `None`。
    pub notes: Option<String>,
}

/// 形状树里的一个节点。
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// 普通文本框(`p:sp` 带 `p:txBody`,无明显几何语义)。
    TextBox(TextFrame),
    /// 表格(`p:graphicFrame` > `a:tbl`)。
    Table(Table),
    /// 图片(`p:pic`)。
    Picture(Picture),
    /// 组合(`p:grpSp`),递归包含子形状。
    Group(Vec<Shape>),
    /// 几何自选图形(`p:sp` 带 `a:prstGeom`)。
    Auto(AutoShape),
    /// 连接线(`p:cxnSp`)。
    Connector(Connector),
    /// 非表格 `p:graphicFrame` 内容(图表 / SmartArt / OLE 等)的占位:内容本身不解析,
    /// 但保留外框矩形与内容种类,供导出侧画占位框 + 告警。
    Placeholder(GraphicPlaceholder),
}

/// 一个文本框体:可选位置 + 段落序列。
#[derive(Debug, Clone, PartialEq)]
pub struct TextFrame {
    pub rect: Option<Rect>,
    pub paragraphs: Vec<Paragraph>,
}

/// 一个段落(`a:p`)。
#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph {
    pub runs: Vec<TextRun>,
    /// 缩进/列表层级(`a:pPr@lvl`),缺省 0。
    pub level: u8,
    /// 对齐方式(`a:pPr@algn`,如 `"ctr"`/`"l"`/`"r"`),原样保留。
    pub align: Option<String>,
}

/// run 的种类:普通文本 / 段内硬换行 / 字段。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RunKind {
    /// 普通文本 run(`a:r`)。
    #[default]
    Text,
    /// 段内硬换行(`a:br`);对应 run 的 `text` 固定为 `"\n"`。
    Break,
    /// 字段 run(`a:fld`,如页码 `slidenum`、日期 `datetime*`);对应 run 的 `text` 是
    /// 文档里缓存的已渲染文本,`field_type` 原样保留 `a:fld@type`。
    Field {
        /// `a:fld@type`(缺失为 `None`)。
        field_type: Option<String>,
    },
}

/// 一段带样式的文字(`a:r` / `a:br` / `a:fld`,由 [`RunKind`] 区分)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextRun {
    pub text: String,
    /// run 种类(文本 / 换行 / 字段),缺省普通文本。
    pub kind: RunKind,
    /// 拉丁字体名(`a:rPr` > `a:latin@typeface`)。
    pub font: Option<String>,
    /// 东亚字体名(`a:rPr` > `a:ea@typeface`,CJK 关键)。
    pub ea_font: Option<String>,
    /// 复杂文种字体名(`a:rPr` > `a:cs@typeface`)。
    pub cs_font: Option<String>,
    /// 字号(磅;OOXML 以百分之磅存储,解析时已除以 100)。
    pub size_pt: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    /// 下划线(`a:rPr@u` 存在且不为 `"none"`)。
    pub underline: bool,
    /// 删除线(`a:rPr@strike` 存在且不为 `"noStrike"`)。
    pub strike: bool,
    /// 纯色填充 RGB(`a:solidFill` > `a:srgbClr`)。
    pub color: Option<Color>,
}

/// 一张表格(`a:tbl`)。
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub rect: Option<Rect>,
    /// 各列宽(EMU,`a:tblGrid` > `a:gridCol@w`,按文档顺序);无 `tblGrid` 时为空。
    pub col_widths: Vec<Emu>,
    pub rows: Vec<Row>,
}

/// 表格的一行(`a:tr`)。
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// 行高(EMU,`a:tr@h`)。
    pub height: Option<Emu>,
}

/// 表格单元格(`a:tc`)。
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub paragraphs: Vec<Paragraph>,
    /// 横向跨列数(`a:tc@gridSpan`),缺省 1。
    pub col_span: u32,
    /// 纵向跨行数(`a:tc@rowSpan`),缺省 1。
    pub row_span: u32,
    /// 单元格纯色填充(`a:tcPr` > `a:solidFill` > `a:srgbClr`)。
    pub fill: Option<Color>,
    /// 是否是被合并掉的延续格(`a:tc@hMerge` / `a:tc@vMerge`)。
    pub merged: bool,
}

/// 一张图片(`p:pic`)。原始字节存放在解析输出的 media map 里,这里只携带定位信息。
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub rect: Option<Rect>,
    /// `a:blip@r:embed` 的关系 id。
    pub rel_id: String,
    /// 经 `.rels` 解析得到的 `ppt/media/*` 文件名(media map 的键)。
    pub media_name: Option<String>,
    /// 图片字节长度(便利字段;字节本身在 media map 里)。
    pub image_bytes_len: usize,
}

/// 几何自选图形(`p:sp` 带 `a:prstGeom`)。
#[derive(Debug, Clone, PartialEq)]
pub struct AutoShape {
    pub rect: Option<Rect>,
    /// 预设几何名(`a:prstGeom@prst`,如 `"rect"`/`"ellipse"`)。
    pub geometry: Option<String>,
    /// 填充色(`spPr` > `a:solidFill` > `a:srgbClr`)。
    pub fill: Option<Color>,
    /// 描边(`spPr` > `a:ln`)。
    pub stroke: Option<Stroke>,
    /// 形状内的文字(若有 `p:txBody`)。
    pub text: Option<TextFrame>,
}

/// 连接线(`p:cxnSp`)—— 形同自选图形,但没有文字体。
#[derive(Debug, Clone, PartialEq)]
pub struct Connector {
    pub rect: Option<Rect>,
    /// 预设几何名(如 `"line"`/`"straightConnector1"`/`"bentConnector3"`)。
    pub geometry: Option<String>,
    /// 填充色(`spPr` > `a:solidFill` > `a:srgbClr`)。
    pub fill: Option<Color>,
    /// 描边(`spPr` > `a:ln`)。
    pub stroke: Option<Stroke>,
}

/// 非表格 `p:graphicFrame`(图表 / SmartArt / OLE 等)的占位信息。
#[derive(Debug, Clone, PartialEq)]
pub struct GraphicPlaceholder {
    /// 外框矩形(`p:graphicFrame` > `p:xfrm`)。
    pub rect: Option<Rect>,
    /// 内容种类:`a:graphicData@uri` 原样(如 `…/chart`、`…/diagram`);缺失为 `None`。
    pub kind: Option<String>,
}

/// 描边属性(`a:ln`):颜色 + 线宽 + 虚线预设。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stroke {
    /// 描边色(`a:ln` > `a:solidFill` > `a:srgbClr`)。
    pub color: Option<Color>,
    /// 线宽(EMU,`a:ln@w`);缺省 `None`。
    pub width_emu: Option<Emu>,
    /// 虚线预设名(`a:prstDash@val`,如 `"dash"`/`"sysDot"`);实线通常缺省为 `None`。
    pub dash: Option<String>,
}

/// 一个 RGB 颜色(来自 `a:srgbClr@val` 的十六进制)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub rgb: [u8; 3],
}

impl Color {
    pub const fn new(rgb: [u8; 3]) -> Self {
        Color { rgb }
    }

    /// 把 `"RRGGBB"` 十六进制串解析成颜色;非法输入返回 `None`。
    pub fn from_hex(hex: &str) -> Option<Self> {
        let h = hex.trim();
        if h.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&h[0..2], 16).ok()?;
        let g = u8::from_str_radix(&h[2..4], 16).ok()?;
        let b = u8::from_str_radix(&h[4..6], 16).ok()?;
        Some(Color { rgb: [r, g, b] })
    }
}
