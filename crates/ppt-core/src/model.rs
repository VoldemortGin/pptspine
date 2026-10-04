//! pptx 结构化解析的结果模型。
//!
//! 目标是**信息无损**:把 OOXML 里的幻灯片 / 文本 / 表格 / 图片 / 自选图形原样搬进
//! 这些朴素的 `struct` / `enum`。本轮不要求 serde,只派生 `Debug`/`Clone`/`PartialEq`。

use crate::color::ColorSpec;
use crate::custgeom::CustGeom;
use crate::diagnostics::Diagnostic;
use crate::geom::{Emu, Rect};
use crate::style::{Caps, PlaceholderRef, ShapeStyle, TextLevelStyle, TextStyleLevels};
use crate::theme::ClrMap;

/// 一份解析好的演示文稿。
#[derive(Debug, Clone, PartialEq)]
pub struct Presentation {
    /// 按 `presentation.xml` 中 `p:sldId` 顺序排列的幻灯片。
    pub slides: Vec<Slide>,
    /// 幻灯片画布尺寸 `(cx, cy)`(EMU,来自 `p:sldSz`)。
    pub slide_size: (Emu, Emu),
    /// 节(`presentation.xml` 扩展 `p14:sectionLst`);无节为空。
    pub sections: Vec<Section>,
    /// 文档属性(`docProps/core.xml` + `docProps/app.xml`);缺失字段为 `None`。
    pub properties: DocProperties,
    /// 首张幻灯片的显示页码(`p:presentation@firstSlideNum`,缺省 1)。
    pub first_slide_num: i32,
    /// 解析诊断:内容被静默丢失 / 降级的结构化事实(截断、嵌套超限、重复引用、缺失部件、
    /// SmartArt / 图表降级);完全正常的文件为空。只含种类 / 部件路径 / 计数,不含正文。
    pub diagnostics: Vec<Diagnostic>,
}

/// 一个节(`p14:section`):名字 + 所含幻灯片的零基序号(按节内顺序)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Section {
    pub name: String,
    pub slide_indices: Vec<usize>,
}

/// 文档属性:`docProps/core.xml`(Dublin Core / OPC core)+ `docProps/app.xml`(扩展属性)。
/// 值一律原样字符串(时间戳保持 W3CDTF 原文);缺失为 `None`。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocProperties {
    /// `dc:title`。
    pub title: Option<String>,
    /// `dc:subject`。
    pub subject: Option<String>,
    /// `dc:creator`。
    pub creator: Option<String>,
    /// `cp:keywords`。
    pub keywords: Option<String>,
    /// `dc:description`。
    pub description: Option<String>,
    /// `cp:category`。
    pub category: Option<String>,
    /// `cp:lastModifiedBy`。
    pub last_modified_by: Option<String>,
    /// `cp:revision`。
    pub revision: Option<String>,
    /// `dcterms:created`(W3CDTF 原文)。
    pub created: Option<String>,
    /// `dcterms:modified`(W3CDTF 原文)。
    pub modified: Option<String>,
    /// `dc:language`。
    pub language: Option<String>,
    /// app.xml `Application`。
    pub application: Option<String>,
    /// app.xml `AppVersion`。
    pub app_version: Option<String>,
    /// app.xml `Company`。
    pub company: Option<String>,
    /// app.xml `Manager`。
    pub manager: Option<String>,
    /// app.xml `PresentationFormat`。
    pub presentation_format: Option<String>,
}

/// 超链接(`a:hlinkClick`,run 级 `a:rPr` 内或形状级 `p:cNvPr` 内)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hyperlink {
    /// `@r:id`(指向所在部件 rels 的一条关系;纯动作链接可缺失)。
    pub rel_id: Option<String>,
    /// `@action`(如 `ppaction://hlinksldjump`、`ppaction://hlinkshowjump?jump=nextslide`)。
    pub action: Option<String>,
    /// `@tooltip`。
    pub tooltip: Option<String>,
    /// 外部链接目标(rels `Target`,仅非 `ppaction://` 链接);内部跳转为 `None`。
    pub url: Option<String>,
    /// 内部跳转的目标幻灯片零基序号(`hlinksldjump` 经 rels 定位,或
    /// `hlinkshowjump` 的 first/last/next/previous 相对当前页计算);解析不出为 `None`。
    pub slide_index: Option<usize>,
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
    /// 幻灯片自身的背景(`p:bg`,§3.o);`None` = 沿 layout → master 链继承。
    pub background: Option<Background>,
    /// 隐藏页(`p:sld@show="0"`);导出侧缺省跳过。
    pub hidden: bool,
    /// `p:sld@showMasterSp`(缺省 true):为 false 时不画 layout / master 的非占位符形状。
    pub show_master_sp: bool,
    /// 批注(旧式 `p:cmLst` 与新式线程批注 `p188:cmLst`,文档顺序;审阅元数据,默认不进
    /// 文本导出,PDF 也不画)。
    pub comments: Vec<Comment>,
}

/// 一条幻灯片批注(或线程批注里的一条回复,此时 `replies` 为空)。属性缺失 → `None`。
/// 作者 / 正文是隐私数据:绝不写进告警 / trace / 日志。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Comment {
    /// 作者名(经 `authorId` 查 `commentAuthors.xml` / `authors.xml`);查不到为 `None`。
    pub author: Option<String>,
    /// 作者缩写。
    pub initials: Option<String>,
    /// 时间戳原文(旧式 `@dt` / 新式 `@created`)。
    pub datetime: Option<String>,
    /// 批注正文(旧式 `p:text`;新式 `p188:txBody` 各段以 `\n` 连接)。
    pub text: Option<String>,
    /// 旧式 `p:pos` 的原始 `(x, y)`(新式批注无此项,为 `None`)。
    pub position: Option<(i64, i64)>,
    /// 新式线程批注的回复(`p188:replyLst`),文档顺序。
    pub replies: Vec<Comment>,
}

/// 幻灯片背景(`p:bg`,§3.o,B-10)。
#[derive(Debug, Clone, PartialEq)]
pub enum Background {
    /// 直接填充(`p:bgPr` 的 solidFill / gradFill / noFill)。
    Fill(Fill),
    /// 图片背景(`p:bgPr > a:blipFill`,rel id 已经部件 rels 折成 media 裸名)。
    Blip { media_name: Option<String> },
    /// 主题引用(`p:bgRef`):`@idx` 1..=999 进 `fillStyleLst[idx-1]`、
    /// 1001.. 进 `bgFillStyleLst[idx-1001]`;子颜色是 `phClr` 的取值。
    Ref { idx: u32, color: Option<ColorSpec> },
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
    /// 组合自身的填充(`p:grpSpPr` 直接子元素;供子形状的 `a:grpFill` 继承)。
    pub fill: Option<Fill>,
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
    /// 图片填充(形状级 `a:blipFill`):图片按形状几何裁剪后画进外框。
    Blip(BlipFill),
    /// 图案填充(`a:pattFill`):前景 / 背景色;渲染降级为两色平均色的纯色。
    Pattern {
        fg: Option<ColorSpec>,
        bg: Option<ColorSpec>,
    },
    /// 继承所在组合的填充(`a:grpFill`);解析终态时沿父组合向上取第一个非 `grpFill` 的填充。
    Group,
}

/// 形状级图片填充(`spPr > a:blipFill`)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BlipFill {
    /// `a:blip@r:embed` 的关系 id。
    pub rel_id: String,
    /// 经 `.rels` 解析得到的 `ppt/media/*` 文件名(media map 的键)。
    pub media_name: Option<String>,
    /// 源裁剪(`a:srcRect`)。
    pub src_rect: Option<RelRect>,
    /// 拉伸目标(`a:stretch > a:fillRect`)。
    pub fill_rect: Option<RelRect>,
    /// 平铺(`a:tile`;渲染 v1 按拉伸画并告警)。
    pub tile: bool,
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

/// `a:bodyPr` 的自动适配子元素(§3.f)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Autofit {
    /// 显式关闭(`a:noAutofit`)。
    None,
    /// 形状适配文字(`a:spAutoFit`;渲染 v1 no-op,PRD §8 B-6)。
    Shape,
    /// 文字缩放适配(`a:normAutofit`):文档里**已存储**的 fontScale /
    /// lnSpcReduction(千分之一个百分点,100000 = 100%;属性缺失 → `None`)。
    Normal {
        font_scale: Option<i64>,
        ln_spc_reduction: Option<i64>,
    },
}

/// 文本体属性(`a:bodyPr`,§3.f):全字段三态(`None` = 缺失 → 沿占位符链继承)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BodyProps {
    /// 垂直锚定(`@anchor`:`t`/`ctr`/`b`/`just`/`dist`)。
    pub anchor: Option<String>,
    /// 锚定居中(`@anchorCtr`,文本块整体水平居中;v1 渲染不消费,信息保留)。
    pub anchor_ctr: Option<bool>,
    /// 左内边距(EMU;OOXML 缺省 91440 = 0.1",解析层不填缺省)。
    pub l_ins: Option<Emu>,
    /// 上内边距(EMU;OOXML 缺省 45720 = 0.05")。
    pub t_ins: Option<Emu>,
    /// 右内边距(EMU)。
    pub r_ins: Option<Emu>,
    /// 下内边距(EMU)。
    pub b_ins: Option<Emu>,
    /// 自动换行(`@wrap`:`"none"` → `Some(false)`,`"square"` → `Some(true)`)。
    pub wrap: Option<bool>,
    /// 文字方向(`@vert`;非 `horz` 渲染侧水平降级 + 告警,PRD §1)。
    pub vert: Option<String>,
    /// 自动适配子元素。
    pub autofit: Option<Autofit>,
}

impl BodyProps {
    /// 逐属性合并:`over`(更近来源)的 `Some` 覆盖 `self` 的对应字段(B-6 链:
    /// master 占位符 → layout 占位符 → 形状自身)。
    #[must_use]
    pub fn overridden_by(&self, over: &BodyProps) -> BodyProps {
        BodyProps {
            anchor: over.anchor.clone().or_else(|| self.anchor.clone()),
            anchor_ctr: over.anchor_ctr.or(self.anchor_ctr),
            l_ins: over.l_ins.or(self.l_ins),
            t_ins: over.t_ins.or(self.t_ins),
            r_ins: over.r_ins.or(self.r_ins),
            b_ins: over.b_ins.or(self.b_ins),
            wrap: over.wrap.or(self.wrap),
            vert: over.vert.clone().or_else(|| self.vert.clone()),
            autofit: over.autofit.or(self.autofit),
        }
    }
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
    /// 文本体属性(`a:bodyPr`,B-6;缺失字段沿占位符链继承)。
    pub body: BodyProps,
    /// 形状级超链接(`p:cNvPr > a:hlinkClick`)。
    pub hyperlink: Option<Hyperlink>,
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

/// run 的种类:普通文本 / 段内硬换行 / 字段 / 公式。
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
    /// 公式 run(`a14:m` 内的 OMML `m:oMathPara` / `m:oMath`):`text` 是线性化后的纯文本
    /// (`1/2`、`x^2`、`sqrt(x)`),无排版;PDF 导出按普通文本画并告警。
    Math,
}

/// 一段带样式的文字(`a:r` / `a:br` / `a:fld`,由 [`RunKind`] 区分)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextRun {
    pub text: String,
    /// run 种类(文本 / 换行 / 字段 / 公式),缺省普通文本。
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
    /// 字符间距(磅,可负;`a:rPr@spc` 以百分之一磅存储,解析时已除以 100)。
    pub char_spacing_pt: Option<f32>,
    /// 上下标基线偏移(相对字号的比例,正上标负下标;`a:rPr@baseline`
    /// 千分之一百分点,解析时已除以 100000)。
    pub baseline: Option<f32>,
    /// 大写变换(`a:rPr@cap`)。
    pub cap: Option<Caps>,
    /// run 级超链接(`a:rPr > a:hlinkClick`)。
    pub hyperlink: Option<Hyperlink>,
}

/// 一张表格(`a:tbl`)。
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub rect: Option<Rect>,
    /// 各列宽(EMU,`a:tblGrid` > `a:gridCol@w`,按文档顺序);无 `tblGrid` 时为空。
    pub col_widths: Vec<Emu>,
    pub rows: Vec<Row>,
    /// 表格样式 id(`a:tblPr > a:tableStyleId`),指向 `ppt/tableStyles.xml` 的
    /// `a:tblStyle@styleId`;找不到时渲染侧降级告警。
    pub table_style_id: Option<String>,
    /// `a:tblPr` 的开关属性(决定表格样式哪些部件生效)。
    pub flags: TableFlags,
}

/// 表格开关属性(`a:tblPr@firstRow/@lastRow/@firstCol/@lastCol/@bandRow/@bandCol`),
/// 决定表格样式的哪些部件生效;缺省全关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TableFlags {
    pub first_row: bool,
    pub last_row: bool,
    pub first_col: bool,
    pub last_col: bool,
    pub band_row: bool,
    pub band_col: bool,
}

/// 表格样式部件的边框(`a:tcStyle > a:tcBdr`)。三态:`None` = 未指定(沿用更低优先级
/// 部件),`Some(None)` = 显式无线(`a:ln > a:noFill`),`Some(Some(_))` = 画线。
/// `left`/`right`/`top`/`bottom` 作用于该部件区域的外沿,`inside_h`/`inside_v` 作用于区域内部
/// 的格间线。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableStyleBorders {
    pub left: Option<Option<Stroke>>,
    pub right: Option<Option<Stroke>>,
    pub top: Option<Option<Stroke>>,
    pub bottom: Option<Option<Stroke>>,
    pub inside_h: Option<Option<Stroke>>,
    pub inside_v: Option<Option<Stroke>>,
}

/// 表格样式的一个部件(`a:wholeTbl` / `a:band1H` / … / `a:lastCol`)。全字段三态,
/// 按优先级逐属性叠加;空部件不产生任何效果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TablePartStyle {
    /// `a:tcStyle > a:fill`:`None` 未指定;`Some(None)` 显式 `a:noFill`;
    /// `Some(Some(_))` 纯色(渐变 / 图案等不支持的填充视为未指定)。
    pub fill: Option<Option<ColorSpec>>,
    /// `a:tcStyle > a:tcBdr`。
    pub borders: TableStyleBorders,
    /// `a:tcTxStyle` 的直接颜色子元素。
    pub text_color: Option<ColorSpec>,
    /// `a:tcTxStyle@b`(`on` / `off`;`def` 或缺失为 `None`)。
    pub bold: Option<bool>,
}

/// 一个表格样式(`ppt/tableStyles.xml` 的 `a:tblStyle`),按部件拆开。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableStyle {
    pub whole_tbl: TablePartStyle,
    pub band1_h: TablePartStyle,
    pub band2_h: TablePartStyle,
    pub band1_v: TablePartStyle,
    pub band2_v: TablePartStyle,
    pub first_row: TablePartStyle,
    pub last_row: TablePartStyle,
    pub first_col: TablePartStyle,
    pub last_col: TablePartStyle,
}

/// 表格的一行(`a:tr`)。
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// 行高(EMU,`a:tr@h`)。
    pub height: Option<Emu>,
}

/// 单元格逐边框线(`a:tcPr > a:lnL/lnR/lnT/lnB`,§3.q)。
/// `None` 边 = 无显式边框(终态取表格样式的对应边,样式也无则不画)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CellBorders {
    pub left: Option<Stroke>,
    pub right: Option<Stroke>,
    pub top: Option<Stroke>,
    pub bottom: Option<Stroke>,
    /// 显式无线(`a:lnL/lnR/lnT/lnB > a:noFill`):该边不画,并压制表格样式的对应边
    /// (此时对应的 `left`/`right`/`top`/`bottom` 为 `None`)。
    pub no_left: bool,
    pub no_right: bool,
    pub no_top: bool,
    pub no_bottom: bool,
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
    /// 显式无填充(`a:tcPr` > `a:noFill`):压制表格样式的填充。
    pub no_fill: bool,
    /// 是否是被合并掉的延续格(`a:tc@hMerge` / `a:tc@vMerge`)。
    pub merged: bool,
    /// 单元格内边距(EMU,`a:tcPr@marL/@marR/@marT/@marB`;缺失 → OOXML 缺省
    /// 91440 / 45720,由解析后的终态 IR 回填)。
    pub mar_l: Option<Emu>,
    pub mar_r: Option<Emu>,
    pub mar_t: Option<Emu>,
    pub mar_b: Option<Emu>,
    /// 垂直锚定(`a:tcPr@anchor`:`t`/`ctr`/`b`)。
    pub anchor: Option<String>,
    /// 逐边框线(§3.q)。
    pub borders: CellBorders,
}

/// 一张图片(`p:pic`)。原始字节存放在解析输出的 media map 里,这里只携带定位信息。
#[derive(Debug, Clone, PartialEq, Default)]
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
    /// 形状名(`p:cNvPr@name`;alt 文本缺失时的回退)。
    pub name: Option<String>,
    /// 替代文本(`p:cNvPr@descr`)。
    pub alt_text: Option<String>,
    /// 标题(`p:cNvPr@title`)。
    pub title: Option<String>,
    /// 形状级超链接(`p:cNvPr > a:hlinkClick`)。
    pub hyperlink: Option<Hyperlink>,
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
    /// 几何来自 `a:custGeom`(渲染侧求值路径;`cust_geom` 为空 = 超预算,按包围盒降级 + 告警)。
    pub custom_geometry: bool,
    /// `a:custGeom` 的参考线 + 路径(超预算 / 空元素为 `None`)。
    pub cust_geom: Option<Box<CustGeom>>,
    /// 形状级超链接(`p:cNvPr > a:hlinkClick`)。
    pub hyperlink: Option<Hyperlink>,
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
    /// 几何来自 `a:custGeom`(`cust_geom` 为空 = 超预算,渲染按缺省直线降级 + 告警)。
    pub custom_geometry: bool,
    /// `a:custGeom` 的参考线 + 路径(超预算 / 空元素为 `None`)。
    pub cust_geom: Option<Box<CustGeom>>,
}

/// 非表格 `p:graphicFrame`(图表 / SmartArt / OLE 等)的占位信息。
#[derive(Debug, Clone, PartialEq)]
pub struct GraphicPlaceholder {
    /// 外框矩形(`p:graphicFrame` > `p:xfrm`)。
    pub rect: Option<Rect>,
    /// 内容种类:`a:graphicData@uri` 原样(如 `…/chart`、`…/diagram`);缺失为 `None`。
    pub kind: Option<String>,
    /// 图表关系 id(`a:graphicData > c:chart@r:id`);非图表为 `None`。
    pub chart_rel_id: Option<String>,
    /// 图表缓存数据(经 slide rels 读 `ppt/charts/chartN.xml`);非图表 / 部件缺失为 `None`。
    pub chart: Option<Chart>,
    /// SmartArt 数据部件关系 id(`a:graphicData > dgm:relIds@r:dm`);非 SmartArt 为 `None`。
    pub diagram_rel_id: Option<String>,
    /// SmartArt 退回 data 部件时抽出的文字(`dgm:pt > dgm:t` 的非空段落,文档顺序);
    /// 有 drawing 部件时 frame 整个替换为组合形状,此字段为空。
    pub diagram_text: Vec<String>,
}

/// 图表种类(`c:plotArea` 下的图类型元素;3D 变体并入同名 2D 种类,`ofPieChart` 并入 `Pie`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChartKind {
    Bar,
    Line,
    Pie,
    Doughnut,
    Area,
    Scatter,
    Bubble,
    Radar,
    Stock,
    Surface,
    /// 未识别的图类型元素(本地名原样,如 `"fooChart"`)。
    Other(String),
}

impl ChartKind {
    /// 由图类型元素本地名(如 `barChart` / `bar3DChart`)识别;非 `*Chart` 元素为 `None`。
    #[must_use]
    pub fn from_element(name: &str) -> Option<Self> {
        let base = name.strip_suffix("Chart")?;
        let base = base.strip_suffix("3D").unwrap_or(base);
        Some(match base {
            "bar" => ChartKind::Bar,
            "line" => ChartKind::Line,
            "pie" | "ofPie" => ChartKind::Pie,
            "doughnut" => ChartKind::Doughnut,
            "area" => ChartKind::Area,
            "scatter" => ChartKind::Scatter,
            "bubble" => ChartKind::Bubble,
            "radar" => ChartKind::Radar,
            "stock" => ChartKind::Stock,
            "surface" => ChartKind::Surface,
            _ => ChartKind::Other(name.to_string()),
        })
    }

    /// 小写种类名(`"bar"` / `"pie"` / …;`Other` 为元素本地名原样)。
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            ChartKind::Bar => "bar",
            ChartKind::Line => "line",
            ChartKind::Pie => "pie",
            ChartKind::Doughnut => "doughnut",
            ChartKind::Area => "area",
            ChartKind::Scatter => "scatter",
            ChartKind::Bubble => "bubble",
            ChartKind::Radar => "radar",
            ChartKind::Stock => "stock",
            ChartKind::Surface => "surface",
            ChartKind::Other(s) => s,
        }
    }
}

/// 图表缓存数据(`c:chartSpace`):只读 `c:strCache` / `c:numCache` / 字面量,**不**读外部工作簿。
#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    /// 主图类型(`c:plotArea` 下第一个图类型;组合图的其余系列一并收进 `series`)。
    pub kind: ChartKind,
    /// 标题纯文本(`c:title > c:tx`;单系列图的自动标题取系列名);无标题为 `None`。
    pub title: Option<String>,
    /// 类别(首个带类别的系列的 `c:cat`;散点 / 气泡图为 `c:xVal`),按点序。
    pub categories: Vec<String>,
    /// 系列,按文档顺序。
    pub series: Vec<ChartSeries>,
    /// 主图类型的 `c:barDir@val`(`"col"` 纵向柱形 / `"bar"` 横向条形);非柱形图或缺失为 `None`。
    pub bar_dir: Option<String>,
    /// 主图类型的 `c:grouping@val`(`clustered` / `stacked` / `percentStacked` / `standard`);
    /// 缺失为 `None`。
    pub grouping: Option<String>,
    /// 主图类型是 3D 变体(如 `bar3DChart`;`kind` 已并入同名 2D 种类)。
    pub three_d: bool,
    /// `c:plotArea` 含多个图类型元素(组合图)。
    pub combo: bool,
    /// 主图类型是 `c:ofPieChart`(复合饼 / 条形饼;`kind` 仍并入 `Pie`)。
    pub of_pie: bool,
    /// 抽取降级告警(缺缓存、点数截断等)。
    pub warnings: Vec<String>,
}

/// 图表的一个系列(`c:ser`)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChartSeries {
    /// 系列名(`c:ser > c:tx`);缺失为 `None`。
    pub name: Option<String>,
    /// 值(`c:val`;散点 / 气泡图为 `c:yVal`),按 `ptCount` 补齐,缺点为 `None`。
    pub values: Vec<Option<f64>>,
    /// 值的数字格式(`c:numCache > c:formatCode`,如 `"General"` / `"0.0%"`)。
    pub format_code: Option<String>,
    /// 系列自带的颜色:柱 / 条 / 面积取 `c:spPr > a:solidFill`,折线取 `c:spPr > a:ln > a:solidFill`;
    /// 缺失 / 渐变 / 图案填充为 `None`(渲染回落主题 accent 循环)。
    pub color: Option<ColorSpec>,
    /// 逐点颜色(`c:dPt@idx` + `c:spPr > a:solidFill`),按文档顺序;饼图的扇区色即来自这里。
    pub point_colors: Vec<(usize, ColorSpec)>,
    /// 生效的数据标签设置(系列级 `c:dLbls` 覆盖图表类型级 `c:dLbls`);都没有为 `None`。
    pub labels: Option<DataLabels>,
}

/// 数据标签设置(`c:dLbls` 的 `c:showVal` / `c:showCatName` / `c:showPercent`;
/// `c:numFmt` 数字格式码不读,标签按默认格式)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DataLabels {
    pub show_val: bool,
    pub show_cat_name: bool,
    pub show_percent: bool,
}

/// 描边属性(`a:ln`):颜色 + 线宽 + 虚线预设 + 两端线端装饰。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stroke {
    /// 显式无线(`a:ln` > `a:noFill`):压制 `lnRef` 主题线,不画描边;此时其余字段均为空。
    pub no_fill: bool,
    /// 描边色(`a:ln` > `a:solidFill`)。
    pub color: Option<ColorSpec>,
    /// 线宽(EMU,`a:ln@w`);缺省 `None`。
    pub width_emu: Option<Emu>,
    /// 虚线预设名(`a:prstDash@val`,如 `"dash"`/`"sysDot"`);实线通常缺省为 `None`。
    pub dash: Option<String>,
    /// 线头装饰(`a:ln > a:headEnd`,路径起点);缺失为 `None`(走 `lnRef` 继承)。
    pub head_end: Option<LineEnd>,
    /// 线尾装饰(`a:ln > a:tailEnd`,路径终点);缺失为 `None`(走 `lnRef` 继承)。
    pub tail_end: Option<LineEnd>,
}

/// 线端装饰种类(`a:headEnd` / `a:tailEnd@type`,ECMA-376 ST_LineEndType)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LineEndKind {
    /// 无装饰(`none`,属性缺省值)。
    #[default]
    None,
    /// 实心三角(`triangle`)。
    Triangle,
    /// 燕尾实心箭头(`stealth`)。
    Stealth,
    /// 实心菱形(`diamond`)。
    Diamond,
    /// 实心椭圆(`oval`)。
    Oval,
    /// 开口箭头(`arrow`,两段线)。
    Arrow,
    /// 规范外取值(原样保留,渲染降级为不画 + 告警)。
    Other(String),
}

/// 线端尺寸档(ECMA-376 ST_LineEndWidth / ST_LineEndLength;缺省 `med`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEndSize {
    /// `sm`。
    Small,
    /// `med`(属性缺省值)。
    #[default]
    Medium,
    /// `lg`。
    Large,
}

/// 一个线端装饰(`a:headEnd` / `a:tailEnd`):种类 + 宽度档(`@w`)+ 长度档(`@len`)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LineEnd {
    pub kind: LineEndKind,
    pub width: LineEndSize,
    pub length: LineEndSize,
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
