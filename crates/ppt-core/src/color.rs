//! DrawingML 颜色:解析级 [`ColorSpec`](srgb / scheme + 修饰变换)与变换数学。
//!
//! 变换语义(ECMA-376 Part 1 §20.1.2.3,与 LibreOffice `oox/source/drawingml/color.cxx`
//! 的实现一致;PowerPoint 未规范给出精确公式,以下为业界共识实现):
//! - `lumMod` / `lumOff` / `satMod`:在**标准 HSL** 空间上,亮度乘 / 亮度加 / 饱和度乘;
//! - `tint` / `shade`:在**线性 sRGB** 空间上(IEC 61966-2-1 线性化)向白 / 向黑插值:
//!   `c' = c·f + (1−f)` / `c' = c·f`;
//! - `alpha`:常数透明度(不改 RGB,随结果带出);
//! - 变换按**文档顺序**依次应用,`f = val / 100000`(千分之一个百分点定点数)。
//!
//! 金标锚点(手算即得 PowerPoint 取色器的真实值):`4472C4` Lighter 40%
//! (`lumMod 60000 + lumOff 40000`)→ `8FAADC`;Darker 25%(`lumMod 75000`)→ `2F5597`。

use crate::model::Color;

/// 一个 DrawingML 颜色修饰变换(`val` 为原始定点数,100000 = 100%)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTransform {
    /// `a:lumMod`:HSL 亮度乘。
    LumMod(i64),
    /// `a:lumOff`:HSL 亮度加。
    LumOff(i64),
    /// `a:tint`:线性 sRGB 向白插值。
    Tint(i64),
    /// `a:shade`:线性 sRGB 向黑缩放。
    Shade(i64),
    /// `a:alpha`:常数透明度。
    Alpha(i64),
    /// `a:satMod`:HSL 饱和度乘(best-effort,PRD §4.1)。
    SatMod(i64),
}

/// 解析级颜色:终端 RGB 或主题 scheme 引用,各自可带修饰变换。
///
/// `a:sysClr` 折算为 [`ColorSpec::Srgb`](取其 `lastClr`,ECMA-376 §20.1.2.3.33 缓存值);
/// scheme 名可为 `dk1`/`lt1`/…/`accent1`/…,也可为 `tx1`/`bg1` 等 clrMap 名或 `phClr`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorSpec {
    /// 显式 RGB(`a:srgbClr@val`,或 `a:sysClr@lastClr`)。
    Srgb {
        rgb: [u8; 3],
        transforms: Vec<ColorTransform>,
    },
    /// 主题 scheme 引用(`a:schemeClr@val`),终端值待 clrMap + clrScheme 解析。
    Scheme {
        name: String,
        transforms: Vec<ColorTransform>,
    },
}

impl ColorSpec {
    /// 无变换的显式 RGB。
    pub const fn srgb(rgb: [u8; 3]) -> Self {
        ColorSpec::Srgb {
            rgb,
            transforms: Vec::new(),
        }
    }

    /// 基础 srgb 值(scheme 引用无终端值 → `None`)。忽略变换——与历史解析行为
    /// (`srgbClr@val` 原样、变换丢弃)保持兼容,供 py-bindings dict 输出复用。
    pub fn base_srgb(&self) -> Option<Color> {
        match self {
            ColorSpec::Srgb { rgb, .. } => Some(Color::new(*rgb)),
            ColorSpec::Scheme { .. } => None,
        }
    }

    /// 该 spec 携带的变换序列。
    pub fn transforms(&self) -> &[ColorTransform] {
        match self {
            ColorSpec::Srgb { transforms, .. } | ColorSpec::Scheme { transforms, .. } => transforms,
        }
    }
}

/// 终端已解析颜色:RGB + 可选常数透明度(来自 `a:alpha`)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedColor {
    pub rgb: [u8; 3],
    /// 常数透明度(0.0 全透明 – 1.0 不透明);无 `alpha` 变换为 `None`。
    pub alpha: Option<f32>,
}

impl ResolvedColor {
    pub const fn opaque(rgb: [u8; 3]) -> Self {
        ResolvedColor { rgb, alpha: None }
    }
}

/// 对基础 RGB 按文档顺序应用一串修饰变换。
pub fn apply_transforms(rgb: [u8; 3], transforms: &[ColorTransform]) -> ResolvedColor {
    let mut c = [
        rgb[0] as f64 / 255.0,
        rgb[1] as f64 / 255.0,
        rgb[2] as f64 / 255.0,
    ];
    let mut alpha: Option<f32> = None;
    for t in transforms {
        match *t {
            ColorTransform::LumMod(v) => {
                let (h, s, l) = rgb_to_hsl(c);
                c = hsl_to_rgb(h, s, clamp01(l * frac(v)));
            }
            ColorTransform::LumOff(v) => {
                let (h, s, l) = rgb_to_hsl(c);
                c = hsl_to_rgb(h, s, clamp01(l + frac(v)));
            }
            ColorTransform::SatMod(v) => {
                let (h, s, l) = rgb_to_hsl(c);
                c = hsl_to_rgb(h, clamp01(s * frac(v)), l);
            }
            ColorTransform::Tint(v) => {
                let f = frac(v);
                c = c.map(|x| clamp01(linear_to_srgb(srgb_to_linear(x) * f + (1.0 - f))));
            }
            ColorTransform::Shade(v) => {
                let f = frac(v);
                c = c.map(|x| clamp01(linear_to_srgb(srgb_to_linear(x) * f)));
            }
            ColorTransform::Alpha(v) => alpha = Some(clamp01(frac(v)) as f32),
        }
    }
    ResolvedColor {
        rgb: c.map(|x| (x * 255.0 + 0.5).floor() as u8),
        alpha,
    }
}

/// 定点数 -> 比例(100000 = 1.0)。
#[inline]
fn frac(val: i64) -> f64 {
    val as f64 / 100_000.0
}

#[inline]
fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

/// sRGB 分量 -> 线性光(IEC 61966-2-1)。
#[inline]
fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// 线性光 -> sRGB 分量(IEC 61966-2-1)。
#[inline]
fn linear_to_srgb(c: f64) -> f64 {
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// 标准 RGB -> HSL(h ∈ [0,1),s/l ∈ [0,1])。
fn rgb_to_hsl(c: [f64; 3]) -> (f64, f64, f64) {
    let [r, g, b] = c;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

/// 标准 HSL -> RGB。
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    if s == 0.0 {
        return [l, l, l];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    [
        hue_to_rgb(p, q, h + 1.0 / 3.0),
        hue_to_rgb(p, q, h),
        hue_to_rgb(p, q, h - 1.0 / 3.0),
    ]
}

fn hue_to_rgb(p: f64, q: f64, t: f64) -> f64 {
    let t = t.rem_euclid(1.0);
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 金标表:手算值(scratchpad `golden_colors.py`,独立实现同一数学),
    /// 其中 Office 常见组合与 PowerPoint 取色器的真实产出十六进制完全一致
    /// (`8FAADC` / `2F5597` / `FBE5D6` 等)。门限 ±2/255(PRD B-8)。
    fn assert_rgb_within(got: [u8; 3], want: [u8; 3]) {
        for i in 0..3 {
            let d = (got[i] as i32 - want[i] as i32).abs();
            assert!(d <= 2, "channel {i}: got {:?}, want {:?}", got, want);
        }
    }

    use ColorTransform::*;

    #[test]
    fn golden_lummod_lumoff_lighter40() {
        // Office "Accent1, Lighter 40%":lumMod 60% + lumOff 40%。
        let got = apply_transforms([0x44, 0x72, 0xC4], &[LumMod(60000), LumOff(40000)]);
        assert_rgb_within(got.rgb, [0x8F, 0xAA, 0xDC]);
        assert_eq!(got.alpha, None);
    }

    #[test]
    fn golden_lummod_darker25() {
        // Office "Accent1, Darker 25%":lumMod 75%。
        let got = apply_transforms([0x44, 0x72, 0xC4], &[LumMod(75000)]);
        assert_rgb_within(got.rgb, [0x2F, 0x55, 0x97]);
    }

    #[test]
    fn golden_lighter80_accent2() {
        // Office "Accent2, Lighter 80%":lumMod 20% + lumOff 80%。
        let got = apply_transforms([0xED, 0x7D, 0x31], &[LumMod(20000), LumOff(80000)]);
        assert_rgb_within(got.rgb, [0xFB, 0xE5, 0xD6]);
    }

    #[test]
    fn golden_tint() {
        let got = apply_transforms([0x44, 0x72, 0xC4], &[Tint(40000)]);
        assert_rgb_within(got.rgb, [0xCF, 0xD5, 0xEA]);
        // 线性空间检查:中灰向白插值(非线性空间会得 0xC0 附近)。
        let grey = apply_transforms([0x80, 0x80, 0x80], &[Tint(50000)]);
        assert_rgb_within(grey.rgb, [0xCD, 0xCD, 0xCD]);
        // 黑的 25% tint。
        let black = apply_transforms([0x00, 0x00, 0x00], &[Tint(25000)]);
        assert_rgb_within(black.rgb, [0xE1, 0xE1, 0xE1]);
    }

    #[test]
    fn golden_shade() {
        let got = apply_transforms([0x44, 0x72, 0xC4], &[Shade(50000)]);
        assert_rgb_within(got.rgb, [0x2F, 0x52, 0x8F]);
        let white = apply_transforms([0xFF, 0xFF, 0xFF], &[Shade(25000)]);
        assert_rgb_within(white.rgb, [0x89, 0x89, 0x89]);
    }

    #[test]
    fn golden_satmod() {
        let got = apply_transforms([0xFF, 0x00, 0x00], &[SatMod(50000)]);
        assert_rgb_within(got.rgb, [0xBF, 0x40, 0x40]);
        let combo = apply_transforms([0x70, 0xAD, 0x47], &[SatMod(150000), LumMod(90000)]);
        assert_rgb_within(combo.rgb, [0x60, 0xB3, 0x29]);
    }

    #[test]
    fn alpha_sets_alpha_keeps_rgb() {
        let got = apply_transforms([0x44, 0x72, 0xC4], &[Alpha(50000)]);
        assert_eq!(got.rgb, [0x44, 0x72, 0xC4]);
        assert_eq!(got.alpha, Some(0.5));
    }

    #[test]
    fn transforms_apply_in_document_order() {
        // 先 tint 后 shade ≠ 先 shade 后 tint。
        let ts = apply_transforms([0x44, 0x72, 0xC4], &[Tint(50000), Shade(50000)]);
        let st = apply_transforms([0x44, 0x72, 0xC4], &[Shade(50000), Tint(50000)]);
        assert_rgb_within(ts.rgb, [0x8D, 0x93, 0xA7]);
        assert_rgb_within(st.rgb, [0xBE, 0xC2, 0xD1]);
        assert_ne!(ts.rgb, st.rgb);
    }

    #[test]
    fn extreme_values_clamp() {
        let got = apply_transforms([0x80, 0x80, 0x80], &[LumOff(200_000)]);
        assert_eq!(got.rgb, [0xFF, 0xFF, 0xFF]);
        let neg = apply_transforms([0x80, 0x80, 0x80], &[LumOff(-200_000)]);
        assert_eq!(neg.rgb, [0x00, 0x00, 0x00]);
    }
}
