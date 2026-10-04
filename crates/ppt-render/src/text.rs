//! 文本框映射(B-2):[`ResolvedTextFrame`] 的段落 / run → `pdf-typeset` 的
//! TS-5 绝对定位文本框输入([`TextBoxSpec`])。
//!
//! 本批语义(与 PRD §8 B-2/B-4 绿条对齐):
//! - 矩形由调用方**预先映射到页坐标 pt**(组合 `Flatten` 预乘,B-5),这里只按
//!   OOXML `bodyPr` 缺省内边距收缩(左右 91440 EMU = 7.2 pt、上下 45720 EMU =
//!   3.6 pt;内边距是文本框自身属性,**不随组合缩放**——与"拆组等价形状"逐字重合)。
//! - 顶部锚定、自动换行、不裁剪(裁剪与 `normAutofit` 一并在 B-6 接线,避免在
//!   bodyPr 未合并前裁掉 PowerPoint 本会自适应的文字)。
//! - 旋转(B-4):`rotation_deg` 直通引擎 `TextBoxSpec::rotation_deg`(视觉逆
//!   时针);pptx `rot`(顺时针 1/60000 度)由调用方换算取负。翻转不镜像文字
//!   (PowerPoint 语义),不进本层。
//! - 项目符号:`buChar` 直接作标签;`buAutoNum` 以逐层计数器格式化(常见 scheme
//!   子集,未知 scheme 退化为 `N.`)。标签字号/字体继承首 run(引擎 `ListLabel`
//!   的语义;`buSzPct`/`buFont` 的独立应用属 B-6)。

use ppt_core::color::ResolvedColor;
use ppt_core::geom::emu_to_points;
use ppt_core::resolved::{
    ResolvedAnchor, ResolvedBodyProps, ResolvedBullet, ResolvedParagraph, ResolvedRun,
};
use ppt_core::style::{Caps, Spacing};

use pdf_typeset::{
    Align, Block, CharacterSpacing, LineSpacing, ListLabel, ParaProps, Rect,
    ResolvedScriptPlacement, Rgb, Run, RunStyle, TextBoxSpec, Typesetter, VAnchor,
};

/// 项目符号标签右缘到正文起点的间距(pt;引擎 `ListLabel::gutter`)。
const BULLET_GUTTER_PT: f64 = 6.0;
/// 继承链全缺 `marL` 时带符号段落的兜底左缩进(228600 EMU = 0.25 in)。
const BULLET_FALLBACK_MARL_PT: f64 = 18.0;
/// 继承链全无字体名时的兜底拉丁字体(Office 缺省主题 minor latin)。
pub(crate) const DEFAULT_LATIN: &str = "Calibri";
/// 上/下标(`baseline` ≠ 0)的字形缩放。OOXML 只给基线偏移、不给缩放比例(PowerPoint
/// 渲染时自行缩小);沿用 docspine 上下标的 0.65(介于 LibreOffice 缺省 58% 与 Office
/// 观感 ≈2/3 之间)。名义字号不变,仍作行高支柱。
const SCRIPT_GLYPH_SCALE: f64 = 0.65;

/// 把一个文本体折成 TS-5 文本框输入。`rect` 已是页坐标 pt(组合仿射预乘后);
/// `rotation_deg` 为视觉逆时针角(pptx `rot` 换算取负后传入)。
pub(crate) fn text_box_spec(
    rect: Rect,
    rotation_deg: f64,
    body: &ResolvedBodyProps,
    paragraphs: &[ResolvedParagraph],
) -> TextBoxSpec {
    let (x, y) = (rect.x0, rect.y0);
    let (w, h) = (rect.x1 - rect.x0, rect.y1 - rect.y0);
    // B-6:bodyPr 内边距(EMU→pt);盒子过小时放弃该向内边距(引擎对 0 宽高再兜底)。
    let (l, r) = (emu_to_points(body.l_ins), emu_to_points(body.r_ins));
    let (t, b) = (emu_to_points(body.t_ins), emu_to_points(body.b_ins));
    let (ix, iw) = if w > l + r {
        (x + l, w - l - r)
    } else {
        (x, w)
    };
    let (iy, ih) = if h > t + b {
        (y + t, h - t - b)
    } else {
        (y, h)
    };

    let mut counters = AutoNumCounters::default();
    let blocks: Vec<Block> = paragraphs
        .iter()
        .map(|p| paragraph_block(p, &mut counters))
        .collect();
    let mut spec = TextBoxSpec::new(Rect::new(ix, iy, ix + iw, iy + ih), blocks);
    spec.rotation_deg = rotation_deg;
    // B-6:垂直锚定 / 自动换行 / normAutofit 字号缩放。
    spec.v_anchor = match body.anchor {
        ResolvedAnchor::Top => VAnchor::Top,
        ResolvedAnchor::Middle => VAnchor::Middle,
        ResolvedAnchor::Bottom => VAnchor::Bottom,
    };
    spec.wrap = body.wrap;
    spec.font_scale = body.font_scale.map(f64::from);
    spec
}

/// B-6 重算式 autofit 的缩放下限(25%)——低于此不再收缩(PowerPoint 下限量级)。
const AUTOFIT_MIN_SCALE: f64 = 0.25;
/// fontScale 逐档步长(每档降 5%,从 95% 起)。
const AUTOFIT_SCALE_STEP_PCT: i32 = 5;
/// `lnSpcReduction` 逐档:0% → 10% → 20%(PowerPoint 上限 20%)。
const AUTOFIT_LN_REDUCTIONS: [f64; 3] = [0.0, 0.10, 0.20];
/// 判定「落入框内」的容差(pt)。
const AUTOFIT_FIT_EPS: f64 = 1e-3;

/// B-6 重算式 autofit:`a:normAutofit` 生效但**未存** `fontScale` 时,按内容重算缩放。
///
/// 用引擎 TS-10 测量 API([`Typesetter::measure_text_box`],与真实排版逐点一致)迭代:
/// 先在 100% 度量,内容不超框则原样返回;否则按 PowerPoint 语义逐档收缩——外层
/// `fontScale` 从 95% 每档降 5% 至 [`AUTOFIT_MIN_SCALE`] 下限,内层 `lnSpcReduction`
/// 0 → 10% → 20%,取**首个**落入框内的组合(字号尽量大、行距压缩尽量小)。缩放**烘进
/// 段落**(返回 spec 的 `font_scale` 保持 `None`),避免引擎再做一轮二分。
///
/// 已存 `fontScale`(stored normAutofit)不走本路径,由 [`text_box_spec`] 原样透传
/// (行为不回归,PRD §9 风险 3)。
pub(crate) fn autofit_text_box_spec(
    ts: &mut Typesetter,
    rect: Rect,
    rotation_deg: f64,
    body: &ResolvedBodyProps,
    paragraphs: &[ResolvedParagraph],
) -> TextBoxSpec {
    let base = text_box_spec(rect, rotation_deg, body, paragraphs);
    let inner_h = (base.rect.y1 - base.rect.y0).abs();
    if ts.measure_text_box(&base).height <= inner_h + AUTOFIT_FIT_EPS {
        return base; // 100% 即落入框内,无需 autofit
    }
    let mut scale_pct = 100 - AUTOFIT_SCALE_STEP_PCT;
    while f64::from(scale_pct) / 100.0 >= AUTOFIT_MIN_SCALE - 1e-9 {
        let scale = f64::from(scale_pct) / 100.0;
        for &red in &AUTOFIT_LN_REDUCTIONS {
            let scaled = scale_paragraphs(paragraphs, scale, red);
            let spec = text_box_spec(rect, rotation_deg, body, &scaled);
            if ts.measure_text_box(&spec).height <= inner_h + AUTOFIT_FIT_EPS {
                return spec;
            }
        }
        scale_pct -= AUTOFIT_SCALE_STEP_PCT;
    }
    // 下限仍溢出:用最小缩放 + 最大行距压缩(内容尽量塞入;残余溢出交引擎裁剪逻辑)。
    let scaled = scale_paragraphs(paragraphs, AUTOFIT_MIN_SCALE, 0.20);
    text_box_spec(rect, rotation_deg, body, &scaled)
}

/// 按 `scale` 缩放各 run 字号、按 `ln_reduction` 压缩行距,返回段落副本。
fn scale_paragraphs(
    paragraphs: &[ResolvedParagraph],
    scale: f64,
    ln_reduction: f64,
) -> Vec<ResolvedParagraph> {
    paragraphs
        .iter()
        .map(|p| {
            let mut np = p.clone();
            for r in &mut np.runs {
                r.size_pt = (f64::from(r.size_pt) * scale) as f32;
                // 与引擎 `font_scale` 收缩同语义:字符间距随字号同比例缩放。
                r.char_spacing_pt = (f64::from(r.char_spacing_pt) * scale) as f32;
            }
            if ln_reduction > 0.0 {
                np.ln_spc = Some(reduce_line_spacing(p.ln_spc, ln_reduction));
            }
            np
        })
        .collect()
}

/// `lnSpcReduction` 语义:百分比行距按 `(1 - red)` 收缩;缺省(单倍)折成 `(1 - red)`
/// 倍;定值(`spcPts`)行距 PowerPoint 不压缩,原样保留。
fn reduce_line_spacing(base: Option<Spacing>, red: f64) -> Spacing {
    match base {
        Some(Spacing::Pts(p)) => Spacing::Pts(p),
        Some(Spacing::Pct(n)) => Spacing::Pct(((n as f64) * (1.0 - red)).round() as i64),
        None => Spacing::Pct(((1.0 - red) * 100_000.0).round() as i64),
    }
}

/// B-6:pptx `a:spcPct`(千分之一百分点)/ `a:spcPts`(点)→ 引擎行距。
fn line_spacing_of(s: Spacing) -> LineSpacing {
    match s {
        Spacing::Pct(n) => LineSpacing::Multiple(n as f64 / 100_000.0),
        Spacing::Pts(p) => LineSpacing::Exact(f64::from(p)),
    }
}

/// 段前 / 段后距(pt)。`spcPts` 直用点值;`spcPct` 按 ECMA-376"相对字号的百分比"
/// 换算:`space = (spcPct / 100000) * font_size_pt`。代表字号由调用方传入(约定取
/// 段落首个 run 的字号;详见 [`paragraph_block`])。
fn space_pts(s: Option<Spacing>, font_size_pt: f32) -> f64 {
    match s {
        Some(Spacing::Pts(p)) => f64::from(p),
        Some(Spacing::Pct(n)) => (n as f64 / 100_000.0) * f64::from(font_size_pt),
        None => 0.0,
    }
}

/// 一个段落 → `Block::Paragraph`。
fn paragraph_block(para: &ResolvedParagraph, counters: &mut AutoNumCounters) -> Block {
    let mut props = ParaProps::new();
    props.align = map_align(para.align.as_deref());
    if let Some(s) = para.ln_spc {
        props.spacing = line_spacing_of(s);
    }
    // 代表字号:`spcPct` 段距按"相对字号的百分比"换算,约定用段落首个 run 的
    // 字号(无 run 时兜底 DEFAULT_FONT_SIZE_PT = 18.0)。
    let rep_size = para
        .runs
        .first()
        .map(|r| r.size_pt)
        .unwrap_or(ppt_core::resolved::DEFAULT_FONT_SIZE_PT);
    props.space_before = space_pts(para.spc_bef, rep_size);
    props.space_after = space_pts(para.spc_aft, rep_size);

    let mar_l = para.mar_l.map(emu_to_points).unwrap_or(0.0);
    let indent = para.indent.map(emu_to_points).unwrap_or(0.0);
    let label = bullet_label(&para.bullet, para.level, counters);
    if label.is_some() {
        // OOXML 语义:带符号段落所有行的正文都从 marL 起;负 indent 只定位符号
        // (引擎标签以右缘贴正文起点绘制,故这里首行缩进归零)。
        props.indent_left = if mar_l > 0.0 {
            mar_l
        } else {
            BULLET_FALLBACK_MARL_PT
        };
        props.first_line_indent = 0.0;
    } else {
        props.indent_left = mar_l;
        props.first_line_indent = indent;
    }
    props.list = label;

    let runs: Vec<Run> = para.runs.iter().map(run_input).collect();
    Block::Paragraph(props, runs)
}

/// 一个 run → 引擎 [`Run`](五属性 + B-3 的下划线/删除线直通 + `spc` / `baseline` /
/// `cap`)。`cap=small` 引擎无小型大写,近似为全大写(降级告警由调用方一次性发出)。
fn run_input(run: &ResolvedRun) -> Run {
    let size = f64::from(run.size_pt);
    let mut style = RunStyle::new(family_for(run), size);
    style.script_placement = script_placement(run.baseline, size);
    style.character_spacing = character_spacing(run.char_spacing_pt);
    style.bold = run.bold;
    style.italic = run.italic;
    style.underline = run.underline;
    style.strike = run.strike;
    style.color = rgb(run.color);
    style.link = run.link.as_deref().and_then(safe_link_uri);
    let text = match run.cap {
        Caps::None => run.text.clone(),
        Caps::Small | Caps::All => run.text.to_uppercase(),
    };
    Run::new(text, style)
}

/// 外链目标 → 可写进 PDF `/URI` 动作的串:只放行 `http` / `https` / `mailto`(scheme 不分大小写),
/// 其余(`javascript:` / `file:` / `data:` / 相对或无 scheme 等)一律 `None`;含空白 / 控制字符、
/// 过长、`http(s)` 缺主机、`mailto` 缺地址的也拒绝;非 ASCII 字符按 UTF-8 百分号编码(PDF URI 须 7 位)。
pub(crate) fn safe_link_uri(url: &str) -> Option<String> {
    /// URI 最大长度(超出视为畸形)。
    const MAX_URI_LEN: usize = 2048;
    let url = url.trim_matches(' ');
    if url.len() > MAX_URI_LEN {
        return None;
    }
    let (scheme, rest) = url.split_once(':')?;
    let rest_ok = match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" => rest.strip_prefix("//").is_some_and(|r| {
            r.chars()
                .next()
                .is_some_and(|c| !matches!(c, '/' | '?' | '#'))
        }),
        "mailto" => !rest.is_empty(),
        _ => false,
    };
    if !rest_ok {
        return None;
    }
    let mut out = String::with_capacity(url.len());
    for c in url.chars() {
        if c.is_ascii_graphic() {
            out.push(c);
        } else if c.is_whitespace() || c.is_control() {
            return None;
        } else {
            let mut buf = [0; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    Some(out)
}

/// `baseline`(相对字号的比例,正上负下)→ 引擎基线偏移:**文档给的偏移量** × 名义
/// 字号(磅,正 = 上抬),字形 ×[`SCRIPT_GLYPH_SCALE`]。0 时 `None`(保持原排版路径,
/// 字节不变);非有限值降级为不偏移,不 panic。
fn script_placement(baseline: f32, size: f64) -> Option<ResolvedScriptPlacement> {
    if baseline == 0.0 || !baseline.is_finite() {
        return None;
    }
    ResolvedScriptPlacement::new(SCRIPT_GLYPH_SCALE, f64::from(baseline) * size).ok()
}

/// `spc`(磅)→ 引擎字符间距:非负走 `new`,负值(紧缩)走 `resolved_signed`。负间距由
/// 引擎 `layout_text_box` / `measure_text_box` 的无错路径预检:可前进的段落照常紧缩,
/// 不可前进的段落只把负间距归零并发 `SignedSpacingFallback` 告警(文本不丢)。
/// 非有限值降级为 0。
fn character_spacing(points: f32) -> CharacterSpacing {
    let points = f64::from(points);
    let spacing = if points < 0.0 {
        CharacterSpacing::resolved_signed(points)
    } else {
        CharacterSpacing::new(points)
    };
    spacing.unwrap_or_default()
}

/// 终端颜色 → 引擎 RGB(文字色的 alpha 引擎 RunStyle 尚不承载,忽略)。
pub(crate) fn rgb(c: ResolvedColor) -> Rgb {
    Rgb::new(
        f64::from(c.rgb[0]) / 255.0,
        f64::from(c.rgb[1]) / 255.0,
        f64::from(c.rgb[2]) / 255.0,
    )
}

/// 选定 run 的请求字体族:CJK 文本优先东亚字体,其余拉丁优先;
/// 链上全缺退 [`DEFAULT_LATIN`](引擎替换表 / 兜底字体继续接力)。
fn family_for(run: &ResolvedRun) -> String {
    let has_cjk = run.text.chars().any(is_cjk);
    let pick = if has_cjk {
        run.ea_font
            .as_deref()
            .or(run.font.as_deref())
            .or(run.cs_font.as_deref())
    } else {
        run.font
            .as_deref()
            .or(run.ea_font.as_deref())
            .or(run.cs_font.as_deref())
    };
    pick.unwrap_or(DEFAULT_LATIN).to_string()
}

/// CJK 判定(统一表意文字 + 扩展 A + 假名 + 谚文 + 全角标点)。
fn is_cjk(ch: char) -> bool {
    matches!(u32::from(ch),
        0x3040..=0x30FF          // 假名
        | 0x3400..=0x4DBF        // 扩展 A
        | 0x4E00..=0x9FFF        // 统一表意
        | 0xAC00..=0xD7AF        // 谚文
        | 0xF900..=0xFAFF        // 兼容表意
        | 0x3000..=0x303F        // CJK 标点
        | 0xFF00..=0xFFEF        // 全角形式
        | 0x20000..=0x2FA1F      // 扩展 B+
    )
}

/// 对齐映射(`a:pPr@algn` → 引擎 [`Align`];`dist` 近似为两端对齐)。
fn map_align(algn: Option<&str>) -> Align {
    match algn {
        Some("ctr") => Align::Center,
        Some("r") => Align::Right,
        Some("just") | Some("dist") => Align::Justify,
        _ => Align::Left,
    }
}

// ---- 项目符号 ---------------------------------------------------------------

/// 每层级一个自动编号计数器(文本框作用域;更深层在出现更浅段落时重置)。
#[derive(Default)]
struct AutoNumCounters {
    counts: Vec<i32>, // 索引 = 层级
}

impl AutoNumCounters {
    /// 该层级下一个序号(`start_at` 只在首次出现时生效),并重置更深层级。
    fn next(&mut self, level: u8, start_at: Option<i32>) -> i32 {
        let lvl = usize::from(level);
        if self.counts.len() <= lvl {
            self.counts.resize(lvl + 1, 0);
        }
        if self.counts[lvl] == 0 {
            self.counts[lvl] = start_at.unwrap_or(1);
        } else {
            self.counts[lvl] += 1;
        }
        self.counts.truncate(lvl + 1);
        self.counts[lvl]
    }
}

/// 项目符号 → 引擎 [`ListLabel`](`None` 含显式 `buNone` 压制)。
fn bullet_label(
    bullet: &ResolvedBullet,
    level: u8,
    counters: &mut AutoNumCounters,
) -> Option<ListLabel> {
    match bullet {
        ResolvedBullet::None => None,
        ResolvedBullet::Char { ch, font, size_pct } => {
            Some(label_with(ch.clone(), font.clone(), *size_pct))
        }
        ResolvedBullet::AutoNum {
            scheme,
            start_at,
            font,
            size_pct,
        } => {
            let n = counters.next(level, *start_at);
            Some(label_with(
                format_autonum(scheme.as_deref(), n),
                font.clone(),
                *size_pct,
            ))
        }
    }
}

/// 组一个引擎 [`ListLabel`]:符号字体 / `buSzPct`(1.0 = 100% → 引擎百分数)直通
/// (TS-8 扩展;`None` 继承首 run)。
fn label_with(text: String, font: Option<String>, size_pct: Option<f32>) -> ListLabel {
    let mut label = ListLabel::new(text, BULLET_GUTTER_PT);
    label.size_pct = size_pct.map(|v| f64::from(v) * 100.0);
    label.font = font;
    label
}

/// 按 `buAutoNum@type` 常见 scheme 子集格式化序号;未知 scheme 退化 `N.`。
fn format_autonum(scheme: Option<&str>, n: i32) -> String {
    let scheme = scheme.unwrap_or("arabicPeriod");
    let base = if scheme.starts_with("alphaLc") {
        to_alpha(n).to_lowercase()
    } else if scheme.starts_with("alphaUc") {
        to_alpha(n)
    } else if scheme.starts_with("romanLc") {
        to_roman(n).to_lowercase()
    } else if scheme.starts_with("romanUc") {
        to_roman(n)
    } else {
        n.to_string()
    };
    if scheme.ends_with("ParenBoth") {
        format!("({base})")
    } else if scheme.ends_with("ParenR") {
        format!("{base})")
    } else if scheme.ends_with("Plain") {
        base
    } else {
        format!("{base}.")
    }
}

/// 1 → A,26 → Z,27 → AA(大写;调用方按 scheme 折小写)。
fn to_alpha(n: i32) -> String {
    let mut n = i64::from(n.max(1));
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(b'A' + u8::try_from(n % 26).unwrap_or(0));
        n /= 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_else(|_| "A".into())
}

/// 1 → I,4 → IV(大写罗马数字;>3999 直接十进制兜底)。
fn to_roman(n: i32) -> String {
    if !(1..=3999).contains(&n) {
        return n.to_string();
    }
    const TABLE: [(i32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut n = n;
    let mut out = String::new();
    for (v, s) in TABLE {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdf_typeset::CharacterSpacing;
    use ppt_core::style::Caps;

    #[test]
    fn autonum_formats() {
        assert_eq!(format_autonum(Some("arabicPeriod"), 3), "3.");
        assert_eq!(format_autonum(Some("arabicParenR"), 2), "2)");
        assert_eq!(format_autonum(Some("arabicParenBoth"), 2), "(2)");
        assert_eq!(format_autonum(Some("arabicPlain"), 7), "7");
        assert_eq!(format_autonum(Some("alphaLcPeriod"), 1), "a.");
        assert_eq!(format_autonum(Some("alphaUcParenR"), 28), "AB)");
        assert_eq!(format_autonum(Some("romanLcPeriod"), 4), "iv.");
        assert_eq!(format_autonum(Some("romanUcPeriod"), 1949), "MCMXLIX.");
        assert_eq!(format_autonum(None, 5), "5.");
    }

    #[test]
    fn autonum_counters_reset_deeper_levels() {
        let mut c = AutoNumCounters::default();
        assert_eq!(c.next(0, None), 1);
        assert_eq!(c.next(1, None), 1);
        assert_eq!(c.next(1, None), 2);
        assert_eq!(c.next(0, None), 2); // 回到 0 层
        assert_eq!(c.next(1, None), 1); // 深层已重置
        let mut d = AutoNumCounters::default();
        assert_eq!(d.next(0, Some(5)), 5);
        assert_eq!(d.next(0, Some(5)), 6);
    }

    #[test]
    fn space_pct_scales_with_font_size() {
        let approx = |a: f64, b: f64| (a - b).abs() < 1e-6;
        // spcPct 相对代表字号换算:50% × 20pt = 10pt;100% × 20pt = 20pt。
        assert!(approx(space_pts(Some(Spacing::Pct(50_000)), 20.0), 10.0));
        assert!(approx(space_pts(Some(Spacing::Pct(100_000)), 20.0), 20.0));
        // spcPts 直用点值,不受字号影响。
        assert!(approx(space_pts(Some(Spacing::Pts(12.0)), 20.0), 12.0));
        // 无声明 → 0。
        assert!(approx(space_pts(None, 20.0), 0.0));
    }

    fn styled_run(text: &str, f: impl FnOnce(&mut ResolvedRun)) -> ResolvedRun {
        let mut run = ResolvedRun {
            text: text.into(),
            kind: ppt_core::model::RunKind::Text,
            font: Some("Arial".into()),
            ea_font: None,
            cs_font: None,
            size_pt: 20.0,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color: ResolvedColor::opaque([0, 0, 0]),
            char_spacing_pt: 0.0,
            baseline: 0.0,
            cap: Caps::None,
            link: None,
        };
        f(&mut run);
        run
    }

    /// `baseline` 正上负下:文档给的偏移比例 × 名义字号 = 基线偏移(磅),字形 ×0.65,
    /// 名义字号不变(行高支柱);0 / 非有限值不设置(原排版路径)。
    #[test]
    fn baseline_maps_to_script_placement() {
        let sup = run_input(&styled_run("sup", |r| r.baseline = 0.30));
        assert_eq!(sup.style.size, 20.0, "名义字号不变");
        let p = sup.style.script_placement.expect("上标有基线偏移");
        assert!((p.glyph_scale() - 0.65).abs() < 1e-9, "字形 ×0.65");
        assert!(
            (p.baseline_shift() - 6.0).abs() < 1e-6,
            "30% × 20pt = 上抬 6pt"
        );

        let sub = run_input(&styled_run("sub", |r| r.baseline = -0.25));
        let p = sub.style.script_placement.expect("下标有基线偏移");
        assert!((p.glyph_scale() - 0.65).abs() < 1e-9);
        assert!(
            (p.baseline_shift() + 5.0).abs() < 1e-6,
            "-25% × 20pt = 下沉 5pt"
        );

        let plain = run_input(&styled_run("plain", |_| {}));
        assert_eq!(plain.style.script_placement, None, "缺省不设置");
        let nan = run_input(&styled_run("nan", |r| r.baseline = f32::NAN));
        assert_eq!(nan.style.script_placement, None, "非有限值降级为不偏移");
    }

    /// `spc`(磅)→ 引擎字符间距:正用 `new`、负用 `resolved_signed`;非有限值降级为 0。
    #[test]
    fn char_spacing_maps_to_engine() {
        let wide = run_input(&styled_run("wide", |r| r.char_spacing_pt = 3.0));
        assert_eq!(wide.style.character_spacing.points(), 3.0);
        assert_eq!(wide.style.script_placement, None, "间距不触发上下标");
        let tight = run_input(&styled_run("tight", |r| r.char_spacing_pt = -1.5));
        assert_eq!(tight.style.character_spacing.points(), -1.5, "负间距保真");
        let plain = run_input(&styled_run("plain", |_| {}));
        assert_eq!(plain.style.character_spacing, CharacterSpacing::default());
        let inf = run_input(&styled_run("inf", |r| r.char_spacing_pt = f32::INFINITY));
        assert_eq!(inf.style.character_spacing, CharacterSpacing::default());
    }

    /// `cap=all` / `cap=small` → 渲染文本大写(small 近似为全大写,引擎无小型大写)。
    #[test]
    fn caps_uppercase_rendered_text() {
        assert_eq!(
            run_input(&styled_run("Mixed", |r| r.cap = Caps::All)).text,
            "MIXED"
        );
        assert_eq!(
            run_input(&styled_run("Mixed", |r| r.cap = Caps::Small)).text,
            "MIXED"
        );
        assert_eq!(run_input(&styled_run("Mixed", |_| {})).text, "Mixed");
    }

    /// 重算式 autofit 缩字号时字符间距同比例缩放(与引擎 `font_scale` 收缩语义一致);
    /// 基线偏移按缩放后字号重算,自然同比例。
    #[test]
    fn autofit_scales_char_spacing() {
        let para = ResolvedParagraph {
            level: 0,
            align: None,
            mar_l: None,
            indent: None,
            ln_spc: None,
            spc_bef: None,
            spc_aft: None,
            bullet: ResolvedBullet::None,
            runs: vec![styled_run("w", |r| {
                r.char_spacing_pt = 4.0;
                r.baseline = 0.30;
            })],
        };
        let scaled = scale_paragraphs(&[para], 0.5, 0.0);
        let r = &scaled[0].runs[0];
        assert!((r.char_spacing_pt - 2.0).abs() < 1e-6);
        assert!((r.size_pt - 10.0).abs() < 1e-6);
        let p = run_input(r).style.script_placement.expect("仍是上标");
        assert!((p.baseline_shift() - 3.0).abs() < 1e-6);
    }

    #[test]
    fn cjk_prefers_ea_font() {
        let run = ResolvedRun {
            text: "中文".into(),
            kind: ppt_core::model::RunKind::Text,
            font: Some("Calibri".into()),
            ea_font: Some("SimSun".into()),
            cs_font: None,
            size_pt: 18.0,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color: ResolvedColor::opaque([0, 0, 0]),
            char_spacing_pt: 0.0,
            baseline: 0.0,
            cap: ppt_core::style::Caps::None,
            link: None,
        };
        assert_eq!(family_for(&run), "SimSun");
    }

    /// 只放行 http / https / mailto;其余 scheme、畸形输入一律不生成链接。
    #[test]
    fn link_uri_allows_only_web_and_mail_schemes() {
        for ok in [
            "http://example.com",
            "https://example.com/a?b=c#d",
            "HTTPS://Example.com/x",
            "  https://example.com/trim  ",
            "mailto:someone@example.com",
            "MailTo:a@b.c?subject=hi",
        ] {
            assert_eq!(safe_link_uri(ok).as_deref(), Some(ok.trim()), "{ok}");
        }
        for bad in [
            "javascript:alert(1)",
            " JavaScript:alert(1)",
            "java\tscript:alert(1)",
            "file:///etc/passwd",
            "data:text/html;base64,AAAA",
            "vbscript:x",
            "ftp://example.com",
            "tel:123",
            "ppaction://hlinksldjump",
            "//example.com",
            "example.com",
            "relative/path.html",
            "http:",
            "http://",
            "https:example.com",
            "mailto:",
            "",
            "https://exa mple.com",
            "https://example.com/\u{0}",
            "https://example.com/\n",
        ] {
            assert_eq!(safe_link_uri(bad), None, "{bad:?}");
        }
        assert_eq!(
            safe_link_uri(&format!("https://e.com/{}", "a".repeat(5000))),
            None,
            "过长"
        );
        assert_eq!(
            safe_link_uri("https://例.com/路径").as_deref(),
            Some("https://%E4%BE%8B.com/%E8%B7%AF%E5%BE%84"),
            "非 ASCII 百分号编码"
        );
    }

    /// run 的外链经 `safe_link_uri` 进入引擎 `RunStyle.link`;被拒的目标不带链接。
    #[test]
    fn run_link_reaches_engine_style_only_when_safe() {
        let ok = run_input(&styled_run("go", |r| {
            r.link = Some("https://example.com".into())
        }));
        assert_eq!(ok.style.link.as_deref(), Some("https://example.com"));
        let bad = run_input(&styled_run("go", |r| {
            r.link = Some("javascript:alert(1)".into())
        }));
        assert_eq!(bad.style.link, None);
        assert_eq!(run_input(&styled_run("go", |_| {})).style.link, None);
    }
}
