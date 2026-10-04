//! 自定义几何(`a:custGeom`)的纯数据模型:参考线 + 路径命令,坐标 / 公式参数保持
//! 原始字符串(数字或参考线名),求值与路径构建在渲染侧(`ppt-render::custgeom`)。
//!
//! 资源上限(对抗输入)在解析期执行:超限整个 `custGeom` 不保留(退回包围盒近似),
//! 并记一条解析诊断。

/// 参考线总数上限(`a:avLst` + `a:gdLst`)。
pub const MAX_GUIDES: usize = 1024;
/// `a:path` 个数上限。
pub const MAX_PATHS: usize = 256;
/// 路径命令总数上限(跨全部 `a:path`)。
pub const MAX_PATH_COMMANDS: usize = 20_000;

/// 一条参考线 `a:gd`:`name` + 公式文本 `fmla`(如 `"*/ w 1 2"`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guide {
    pub name: String,
    pub fmla: String,
}

/// 路径点:`x` / `y` 各是数字字面量或参考线名(原样保存,求值时再解析)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustPt {
    pub x: String,
    pub y: String,
}

/// 路径命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathCmd {
    MoveTo(CustPt),
    LnTo(CustPt),
    CubicBezTo(CustPt, CustPt, CustPt),
    QuadBezTo(CustPt, CustPt),
    /// `a:arcTo`:椭圆半径 `wR` / `hR`、起始角 `stAng`、扫过角 `swAng`(角度 1/60000 度)。
    ArcTo {
        w_r: String,
        h_r: String,
        st_ang: String,
        sw_ang: String,
    },
    Close,
}

/// `a:path@fill`。明暗变体(lighten / darken …)渲染时按 `Norm` 画。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PathFill {
    #[default]
    Norm,
    None,
    Lighten,
    LightenLess,
    Darken,
    DarkenLess,
}

/// 一条 `a:path`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustPath {
    /// 路径坐标系宽 / 高(`@w` / `@h`);缺失或 0 = 用形状自身尺寸。
    pub w: Option<i64>,
    pub h: Option<i64>,
    pub fill: PathFill,
    /// `@stroke`(缺省 true)。
    pub stroke: bool,
    pub extrusion_ok: bool,
    pub cmds: Vec<PathCmd>,
}

/// 自定义几何。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CustGeom {
    /// `a:avLst`(先于 `gdLst` 求值)。
    pub av_lst: Vec<Guide>,
    pub gd_lst: Vec<Guide>,
    pub paths: Vec<CustPath>,
}
