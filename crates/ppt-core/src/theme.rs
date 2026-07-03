//! 主题子系统的数据类型(`ppt/theme/themeN.xml`,ECMA-376 §20.1.6):
//! 12 色 `clrScheme`、`fontScheme`(major/minor 三路字体)、`clrMap` 重映射,
//! 以及 `fmtScheme` 的填充 / 线条格式列表(供 `p:style` fillRef/lnRef 解析)。
//! 纯数据,无 IO / XML —— 由 `ppt-parse` 填充。

use crate::color::ColorSpec;
use crate::geom::Emu;
use crate::model::Color;

/// 主题 12 色方案(`a:clrScheme`)。槽位值已是终端 RGB
/// (`sysClr` 在解析期折算为其 `lastClr` 缓存值)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorScheme {
    pub dk1: Color,
    pub lt1: Color,
    pub dk2: Color,
    pub lt2: Color,
    pub accent1: Color,
    pub accent2: Color,
    pub accent3: Color,
    pub accent4: Color,
    pub accent5: Color,
    pub accent6: Color,
    pub hlink: Color,
    pub fol_hlink: Color,
}

impl Default for ColorScheme {
    /// 无主题时的兜底:黑字白底,其余槽位黑(确定性、绝不 panic)。
    fn default() -> Self {
        let black = Color::new([0, 0, 0]);
        let white = Color::new([255, 255, 255]);
        ColorScheme {
            dk1: black,
            lt1: white,
            dk2: black,
            lt2: white,
            accent1: black,
            accent2: black,
            accent3: black,
            accent4: black,
            accent5: black,
            accent6: black,
            hlink: black,
            fol_hlink: black,
        }
    }
}

impl ColorScheme {
    /// 按 scheme 槽位名取终端色(`dk1`/`lt1`/…/`accent6`/`hlink`/`folHlink`)。
    /// 未知槽位名返回 `None`。
    pub fn get(&self, slot: &str) -> Option<Color> {
        Some(match slot {
            "dk1" => self.dk1,
            "lt1" => self.lt1,
            "dk2" => self.dk2,
            "lt2" => self.lt2,
            "accent1" => self.accent1,
            "accent2" => self.accent2,
            "accent3" => self.accent3,
            "accent4" => self.accent4,
            "accent5" => self.accent5,
            "accent6" => self.accent6,
            "hlink" => self.hlink,
            "folHlink" => self.fol_hlink,
            _ => return None,
        })
    }
}

/// 一路字体集合(`a:majorFont` / `a:minorFont` 里的 latin/ea/cs `@typeface`;
/// 空串按缺省处理为 `None`)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FontSet {
    pub latin: Option<String>,
    pub ea: Option<String>,
    pub cs: Option<String>,
}

/// 主题字体方案(`a:fontScheme`)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FontScheme {
    pub major: FontSet,
    pub minor: FontSet,
}

/// 颜色映射(`p:clrMap` / `a:overrideClrMapping` 的 12 个属性):把 `bg1`/`tx1` 等
/// 映射名转到 `dk1`/`lt1` 等 scheme 槽位(ECMA-376 §19.3.1.6)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClrMap {
    pub bg1: String,
    pub tx1: String,
    pub bg2: String,
    pub tx2: String,
    pub accent1: String,
    pub accent2: String,
    pub accent3: String,
    pub accent4: String,
    pub accent5: String,
    pub accent6: String,
    pub hlink: String,
    pub fol_hlink: String,
}

impl Default for ClrMap {
    /// PowerPoint 惯例缺省映射:`bg1→lt1, tx1→dk1, bg2→lt2, tx2→dk2`,其余恒等。
    fn default() -> Self {
        ClrMap {
            bg1: "lt1".into(),
            tx1: "dk1".into(),
            bg2: "lt2".into(),
            tx2: "dk2".into(),
            accent1: "accent1".into(),
            accent2: "accent2".into(),
            accent3: "accent3".into(),
            accent4: "accent4".into(),
            accent5: "accent5".into(),
            accent6: "accent6".into(),
            hlink: "hlink".into(),
            fol_hlink: "folHlink".into(),
        }
    }
}

impl ClrMap {
    /// 把一个 `schemeClr@val` 名重映射到 scheme 槽位名:映射名(`tx1`/`bg1` 等)
    /// 经本表转换;已是槽位名(`dk1`/`accent1` 等)原样返回。
    pub fn map<'a>(&'a self, name: &'a str) -> &'a str {
        match name {
            "bg1" => &self.bg1,
            "tx1" => &self.tx1,
            "bg2" => &self.bg2,
            "tx2" => &self.tx2,
            "accent1" => &self.accent1,
            "accent2" => &self.accent2,
            "accent3" => &self.accent3,
            "accent4" => &self.accent4,
            "accent5" => &self.accent5,
            "accent6" => &self.accent6,
            "hlink" => &self.hlink,
            "folHlink" => &self.fol_hlink,
            other => other,
        }
    }
}

/// `fmtScheme > a:lnStyleLst` 的一项:主题线条(宽度 + 颜色 spec,`phClr` 待
/// `lnRef` 提供占位色)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThemeLine {
    pub color: Option<ColorSpec>,
    pub width_emu: Option<Emu>,
}

/// 一份主题(`a:theme > a:themeElements`)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Theme {
    pub color_scheme: ColorScheme,
    pub font_scheme: FontScheme,
    /// `fmtScheme > a:fillStyleLst` 各项的纯色 spec(1 基 `fillRef@idx` 对应
    /// `fill_styles[idx-1]`);非纯色项(渐变等)为 `None`(降级为代表色,PRD §4.1)。
    pub fill_styles: Vec<Option<ColorSpec>>,
    /// `fmtScheme > a:lnStyleLst` 各项(1 基 `lnRef@idx` 对应 `line_styles[idx-1]`)。
    pub line_styles: Vec<ThemeLine>,
    /// `fmtScheme > a:bgFillStyleLst` 各项的纯色 spec(`p:bgRef@idx` ≥ 1001 对应
    /// `bg_fill_styles[idx-1001]`,B-10);非纯色项为 `None`(降级为代表色)。
    pub bg_fill_styles: Vec<Option<ColorSpec>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_clr_map_maps_text_and_background() {
        let m = ClrMap::default();
        assert_eq!(m.map("tx1"), "dk1");
        assert_eq!(m.map("bg1"), "lt1");
        assert_eq!(m.map("tx2"), "dk2");
        assert_eq!(m.map("bg2"), "lt2");
        // 已是槽位名的原样通过。
        assert_eq!(m.map("dk1"), "dk1");
        assert_eq!(m.map("accent3"), "accent3");
    }

    #[test]
    fn color_scheme_lookup() {
        let s = ColorScheme {
            accent1: Color::new([0x44, 0x72, 0xC4]),
            ..ColorScheme::default()
        };
        assert_eq!(s.get("accent1"), Some(Color::new([0x44, 0x72, 0xC4])));
        assert_eq!(s.get("nope"), None);
    }
}
