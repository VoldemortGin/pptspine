//! pptx 结构化解析的结果模型。
//!
//! 目标是**信息无损**:把 OOXML 里的幻灯片 / 文本 / 表格 / 图片 / 自选图形原样搬进
//! 这些朴素的 `struct` / `enum`。本轮不要求 serde,只派生 `Debug`/`Clone`/`PartialEq`。

use crate::color::ColorSpec;
use crate::geom::{Emu, Rect};
use crate::style::{PlaceholderRef, ShapeStyle, TextLevelStyle, TextStyleLevels};
use crate::theme::ClrMap;

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
    /// 颜色映射覆盖(`p:clrMapOvr > a:overrideClrMapping`);
    /// `None` = 沿用 layout / master 的映射(`a:masterClrMapping` 或缺失)。
    pub clr_map_ovr: Option<ClrMap>,
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
    /// 组合(`p:grpSp`),携带自身变换 + 子坐标空间,递归包含子形状。
    Group(GroupShape),
    /// 几何自选图形(`p:sp` 带 `a:prstGeom`)。
    Auto(AutoShape),
    /// 连接线(`p:cxnSp`)。
    Connector(Connector),
    /// 非表格 `p:graphicFrame` 内容(图表 / SmartArt / OLE 等)的占位:内容本身不解析,
    /// 但保留外框矩形与内容种类,供导出侧画占位框 + 告警。
    Placeholder(GraphicPlaceholder),
}

/// `a:xfrm` 自身属性(§3.d):旋转 + 翻转(矩形之外的变换部分)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Xfrm {
    /// 旋转角(1/60000 度,顺时针为正;`a:xfrm@rot`)。
    pub rot: i32,
    /// 水平翻转(`a:xfrm@flipH`)。
    pub flip_h: bool,
    /// 垂直翻转(`a:xfrm@flipV`)。
    pub flip_v: bool,
}

impl Xfrm {
    /// 是否恒等变换(无旋转、无翻转)。
    #[must_use]
    pub fn is_identity(self) -> bool {
        self.rot == 0 && !self.flip_h && !self.flip_v
    }

    /// 旋转角(度,顺时针为正)。
    #[must_use]
    pub fn rot_deg(self) -> f64 {
        f64::from(self.rot) / 60_000.0
    }
}

/// 组合(`p:grpSp`,§3.e):自身矩形(`grpSpPr > a:xfrm` off/ext)+ 子坐标空间
/// (`a:chOff`/`a:chExt`)+ 旋转/翻转 + 子形状。渲染按
/// `(child − chOff) · (ext/chExt) + off` 重映射子坐标(B-5)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GroupShape {
    /// 组合在父坐标系里的矩形(`a:off`/`a:ext`)。
    pub rect: Option<Rect>,
    /// 子坐标空间(`a:chOff`/`a:chExt`)。
    pub child_rect: Option<Rect>,
    /// 组合自身的旋转/翻转。
    pub xfrm: Xfrm,
    /// 子形状,按文档顺序。
    pub children: Vec<Shape>,
}

/// 形状级填充(`spPr` 直接子元素,§3.m):显式 `a:noFill` 与"未设置(走继承 /
/// `p:style` 引用)"区分开。
#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    /// 显式无填充(`a:noFill`)。
    None,
    /// 纯色(`a:solidFill`)。
    Solid(ColorSpec),
    /// 渐变(`a:gradFill`):stop 颜色按文档顺序(v1 渲染降级取首个作代表色)。
    Gradient(Vec<ColorSpec>),
    /// 图片填充(形状级 `a:blipFill`;v1 渲染不涂,信息保留)。
    Blip,
}

/// 相对矩形(`a:srcRect` / `a:fillRect`,§3.n):四边偏移,单位千分之一百分点
/// (100000 = 100%);正值向内收,负值向外扩。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RelRect {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

/// 一个文本框体:可选位置 + 段落序列(+ 继承链所需的占位符 / 列表样式 / 形状样式引用)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextFrame {
    pub rect: Option<Rect>,
    /// 旋转/翻转(`a:xfrm` 自身属性)。
    pub xfrm: Xfrm,
    pub paragraphs: Vec<Paragraph>,
    /// 占位符标识(`p:nvSpPr > p:nvPr > p:ph`);非占位符为 `None`。
    pub placeholder: Option<PlaceholderRef>,
    /// 本形状 `txBody` 自带的 `a:lstStyle`(继承链一环);缺失为 `None`。
    pub list_style: Option<TextStyleLevels>,
    /// 形状样式引用(`p:style`,主题索引式格式)。
    pub style: Option<ShapeStyle>,
}

/// 一个段落(`a:p`)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Paragraph {
    pub runs: Vec<TextRun>,
    /// 缩进/列表层级(`a:pPr@lvl`),缺省 0。
    pub level: u8,
    /// 对齐方式(`a:pPr@algn`,如 `"ctr"`/`"l"`/`"r"`),原样保留(镜像 `props.align`)。
    pub align: Option<String>,
    /// 段落直接格式化的完整 `a:pPr`(对齐 / 列表缩进 / 项目符号 / `defRPr`),
    /// 继承链的最近段落级来源。
    pub props: TextLevelStyle,
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
    /// 三态:`None` = 属性缺失(继承),`Some(v)` = 显式开 / 关。以下同。
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    /// 下划线(`a:rPr@u`;`Some(false)` 即显式 `u="none"`)。
    pub underline: Option<bool>,
    /// 删除线(`a:rPr@strike`;`Some(false)` 即显式 `strike="noStrike"`)。
    pub strike: Option<bool>,
    /// 纯色填充(`a:solidFill`;srgb / scheme + 变换,见 [`ColorSpec`])。
    pub color: Option<ColorSpec>,
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
    /// 单元格纯色填充(`a:tcPr` > `a:solidFill`)。
    pub fill: Option<ColorSpec>,
    /// 是否是被合并掉的延续格(`a:tc@hMerge` / `a:tc@vMerge`)。
    pub merged: bool,
}

/// 一张图片(`p:pic`)。原始字节存放在解析输出的 media map 里,这里只携带定位信息。
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub rect: Option<Rect>,
    /// 旋转/翻转(`a:xfrm` 自身属性;翻转对图片是真镜像)。
    pub xfrm: Xfrm,
    /// `a:blip@r:embed` 的关系 id。
    pub rel_id: String,
    /// 经 `.rels` 解析得到的 `ppt/media/*` 文件名(media map 的键)。
    pub media_name: Option<String>,
    /// 图片字节长度(便利字段;字节本身在 media map 里)。
    pub image_bytes_len: usize,
    /// 源裁剪(`a:blipFill > a:srcRect`)。
    pub src_rect: Option<RelRect>,
    /// 拉伸目标(`a:blipFill > a:stretch > a:fillRect`)。
    pub fill_rect: Option<RelRect>,
    /// 占位符标识(`p:nvPicPr > p:nvPr > p:ph`,图片占位符几何可继承)。
    pub placeholder: Option<PlaceholderRef>,
}

/// 几何自选图形(`p:sp` 带 `a:prstGeom`)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AutoShape {
    pub rect: Option<Rect>,
    /// 旋转/翻转(`a:xfrm` 自身属性)。
    pub xfrm: Xfrm,
    /// 预设几何名(`a:prstGeom@prst`,如 `"rect"`/`"ellipse"`)。
    pub geometry: Option<String>,
    /// 预设几何调整值(`a:avLst > a:gd`,`(name, val)` 对,§3.j)。
    pub adjusts: Vec<(String, i64)>,
    /// 填充(`spPr` 直接子元素;`None` = 未设置,走继承 / 样式引用)。
    pub fill: Option<Fill>,
    /// 描边(`spPr` > `a:ln`)。
    pub stroke: Option<Stroke>,
    /// 形状内的文字(若有 `p:txBody`)。装箱以控制 `Shape` 枚举体积
    /// (clippy `large_enum_variant`)。
    pub text: Option<Box<TextFrame>>,
    /// 占位符标识(`p:nvSpPr > p:nvPr > p:ph`)。
    pub placeholder: Option<PlaceholderRef>,
    /// 形状样式引用(`p:style`)。
    pub style: Option<ShapeStyle>,
}

/// 连接线(`p:cxnSp`)—— 形同自选图形,但没有文字体。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Connector {
    pub rect: Option<Rect>,
    /// 旋转/翻转(`a:xfrm` 自身属性;连接线方向常靠翻转表达)。
    pub xfrm: Xfrm,
    /// 预设几何名(如 `"line"`/`"straightConnector1"`/`"bentConnector3"`)。
    pub geometry: Option<String>,
    /// 预设几何调整值(`a:avLst > a:gd`,如 bentConnector3 的转折位置)。
    pub adjusts: Vec<(String, i64)>,
    /// 填充(`spPr` 直接子元素;`None` = 未设置,走继承 / 样式引用)。
    pub fill: Option<Fill>,
    /// 描边(`spPr` > `a:ln`)。
    pub stroke: Option<Stroke>,
    /// 形状样式引用(`p:style`,连接线常经 `lnRef` 取主题线色)。
    pub style: Option<ShapeStyle>,
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
    /// 描边色(`a:ln` > `a:solidFill`)。
    pub color: Option<ColorSpec>,
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
