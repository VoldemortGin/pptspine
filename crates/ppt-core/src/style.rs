//! 文本样式继承链的数据类型(PRD-PDF-EXPORT §4.1,B-9):
//! 占位符引用、层级列表样式(`a:lstStyle` / master `p:txStyles`)、可继承的
//! run / 段落属性(全 `Option` 三态,便于逐属性合并),以及 `p:style` 形状样式引用。
//! 纯数据 + 合并逻辑,无 IO / XML。

use crate::color::ColorSpec;
use crate::geom::Emu;

/// 占位符标识(`p:nvSpPr > p:nvPr > p:ph`)。
///
/// `kind` 缺省语义为 `body`(ECMA-376 §19.3.1.36 `type` 默认值);`idx` 缺省按 0 匹配。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlaceholderRef {
    /// `@type`(如 `title`/`ctrTitle`/`body`/`subTitle`/`dt`/`ftr`/`sldNum`)。
    pub kind: Option<String>,
    /// `@idx`。
    pub idx: Option<u32>,
}

/// 项目符号(`a:buNone` / `a:buChar` / `a:buAutoNum`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bullet {
    /// 显式无符号(`buNone`)——在更近层级出现时**压制**继承来的符号。
    None,
    /// 字符符号(`buChar@char`)。
    Char(String),
    /// 自动编号(`buAutoNum@type/@startAt`)。
    AutoNum {
        scheme: Option<String>,
        start_at: Option<i32>,
    },
}

/// 可继承的 run 级样式(`a:rPr` / `a:defRPr` 形):全字段三态
/// (`None` = 未指定 → 继承;`Some` = 显式指定 → 覆盖)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunStyle {
    pub size_pt: Option<f32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub font: Option<String>,
    pub ea_font: Option<String>,
    pub cs_font: Option<String>,
    pub color: Option<ColorSpec>,
}

impl RunStyle {
    /// 逐属性合并:`over`(更近来源)的 `Some` 覆盖 `self` 的对应字段。
    pub fn overridden_by(&self, over: &RunStyle) -> RunStyle {
        RunStyle {
            size_pt: over.size_pt.or(self.size_pt),
            bold: over.bold.or(self.bold),
            italic: over.italic.or(self.italic),
            underline: over.underline.or(self.underline),
            strike: over.strike.or(self.strike),
            font: over.font.clone().or_else(|| self.font.clone()),
            ea_font: over.ea_font.clone().or_else(|| self.ea_font.clone()),
            cs_font: over.cs_font.clone().or_else(|| self.cs_font.clone()),
            color: over.color.clone().or_else(|| self.color.clone()),
        }
    }
}

/// 一个层级的段落样式(`a:lvlNpPr` / 段落自身 `a:pPr` 形):
/// 对齐、列表缩进、项目符号(字符 / 字体 / 大小)与缺省 run 样式。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextLevelStyle {
    /// 对齐(`@algn`)。
    pub align: Option<String>,
    /// 左边距(EMU,`@marL`)。
    pub mar_l: Option<Emu>,
    /// 首行缩进(EMU,`@indent`,可负)。
    pub indent: Option<Emu>,
    /// 项目符号(`None` = 未指定 → 继承)。
    pub bullet: Option<Bullet>,
    /// 符号字体(`a:buFont@typeface`)。
    pub bu_font: Option<String>,
    /// 符号大小(千分之一个百分点,`a:buSzPct@val`,100000 = 100%)。
    pub bu_size_pct: Option<i64>,
    /// 缺省 run 样式(`a:defRPr`)。
    pub def_rpr: Option<RunStyle>,
}

impl TextLevelStyle {
    /// 逐属性合并;`def_rpr` 递归合并(双方都有时按属性覆盖)。
    pub fn overridden_by(&self, over: &TextLevelStyle) -> TextLevelStyle {
        TextLevelStyle {
            align: over.align.clone().or_else(|| self.align.clone()),
            mar_l: over.mar_l.or(self.mar_l),
            indent: over.indent.or(self.indent),
            bullet: over.bullet.clone().or_else(|| self.bullet.clone()),
            bu_font: over.bu_font.clone().or_else(|| self.bu_font.clone()),
            bu_size_pct: over.bu_size_pct.or(self.bu_size_pct),
            def_rpr: match (&self.def_rpr, &over.def_rpr) {
                (Some(base), Some(o)) => Some(base.overridden_by(o)),
                (base, o) => o.clone().or_else(|| base.clone()),
            },
        }
    }
}

/// 层级列表样式(`a:lstStyle` / master `p:txStyles` 一桶):
/// 9 层(`lvl1pPr`…`lvl9pPr`,0 基下标 = `pPr@lvl`)。
/// 层数组装箱,避免把携带它的形状模型撑大(clippy `large_enum_variant`)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextStyleLevels {
    pub levels: Box<[Option<TextLevelStyle>; 9]>,
}

impl TextStyleLevels {
    /// 取某层样式(`lvl` 0 基,越界收敛到第 9 层)。
    pub fn level(&self, lvl: u8) -> Option<&TextLevelStyle> {
        self.levels[usize::from(lvl).min(8)].as_ref()
    }

    /// 是否一层都没有(便于把空 `lstStyle` 当 `None` 用)。
    pub fn is_empty(&self) -> bool {
        self.levels.iter().all(|l| l.is_none())
    }
}

/// master `p:txStyles` 的三桶(占位符种类 → 桶,PRD §4.1)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TxStyles {
    pub title: TextStyleLevels,
    pub body: TextStyleLevels,
    pub other: TextStyleLevels,
}

/// `p:style` 里的一个格式列表引用(`a:fillRef` / `a:lnRef`):
/// 1 基 `@idx` 指进主题 `fmtScheme` 列表,子颜色是 `phClr` 的取值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleMatrixRef {
    pub idx: u32,
    pub color: Option<ColorSpec>,
}

/// `p:style > a:fontRef`:`@idx` ∈ `major`/`minor`/`none`,子颜色为文字缺省色。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontRef {
    pub idx: String,
    pub color: Option<ColorSpec>,
}

/// 形状样式引用集(`p:style`,主题索引式格式;ECMA-376 §19.3.1.46)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShapeStyle {
    pub fill_ref: Option<StyleMatrixRef>,
    pub ln_ref: Option<StyleMatrixRef>,
    pub font_ref: Option<FontRef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_style_merges_per_attribute() {
        let base = RunStyle {
            size_pt: Some(44.0),
            bold: Some(true),
            font: Some("+mj-lt".into()),
            ..RunStyle::default()
        };
        let over = RunStyle {
            size_pt: Some(32.0),
            italic: Some(true),
            ..RunStyle::default()
        };
        let merged = base.overridden_by(&over);
        assert_eq!(merged.size_pt, Some(32.0)); // 更近来源赢
        assert_eq!(merged.bold, Some(true)); // 未指定 → 继承
        assert_eq!(merged.italic, Some(true));
        assert_eq!(merged.font.as_deref(), Some("+mj-lt"));
    }

    #[test]
    fn level_style_merges_nested_def_rpr() {
        let base = TextLevelStyle {
            bullet: Some(Bullet::Char("•".into())),
            def_rpr: Some(RunStyle {
                size_pt: Some(28.0),
                bold: Some(true),
                ..RunStyle::default()
            }),
            ..TextLevelStyle::default()
        };
        let over = TextLevelStyle {
            bullet: Some(Bullet::None), // 更近层显式压制
            def_rpr: Some(RunStyle {
                size_pt: Some(20.0),
                ..RunStyle::default()
            }),
            ..TextLevelStyle::default()
        };
        let merged = base.overridden_by(&over);
        assert_eq!(merged.bullet, Some(Bullet::None));
        let rpr = merged.def_rpr.expect("merged def_rpr");
        assert_eq!(rpr.size_pt, Some(20.0));
        assert_eq!(rpr.bold, Some(true)); // 深层字段仍继承
    }

    #[test]
    fn levels_clamp_to_ninth() {
        let mut ls = TextStyleLevels::default();
        ls.levels[8] = Some(TextLevelStyle {
            align: Some("r".into()),
            ..TextLevelStyle::default()
        });
        assert_eq!(ls.level(20).and_then(|l| l.align.as_deref()), Some("r"));
        assert!(ls.level(0).is_none());
    }
}
