//! 图表矢量渲染:[`Chart`] 缓存数据 → 基础矢量图(簇状 / 堆积 / 百分比堆积的柱形与条形、
//! 折线、饼图),替换图表占位框。
//!
//! 分两层:
//! - **几何**([`layout`] 及其辅助):纯函数,数据 + 外框(页坐标 pt,左上原点、y 向下)→
//!   一组矩形 / 折线 / 标记点 / 扇形 / 网格线 / 文字框;不碰引擎、无随机、无时间、无哈希序,
//!   同输入同输出。文字宽度用字号 × 字符数的确定性估算(只用于留白与标签抽稀)。
//! - **落 op**([`chart_ops`]):把几何映射到 `pdf-typeset` 现有绘制 op(`FillRect` / `Line` /
//!   `Path`(扇形用三次贝塞尔逼近圆弧)/ `FillCircle` / 文本框)。
//!
//! 不支持的图(面积 / 散点 / 雷达 / 圆环 / 组合 / 3D 等)、无可绘数据、点数过多或外框过小:
//! 返回 [`Unsupported`],调用方保留占位框并记 `chart-degraded` 告警。
//! 数值的 NaN / ±∞ 视为缺点;绝对值超过 [`VALUE_LIMIT`] 的值截断到 ±[`VALUE_LIMIT`]。

use ppt_core::model::{Chart, ChartKind, ChartSeries};

use pdf_typeset::{
    Align, Block, ExportWarning, Fill, LineCap, LineJoin, Op, ParaProps, PathSeg, Rect, Rgb, Run,
    RunStyle, Stroke, TextBoxSpec, Typesetter, VAnchor,
};

use crate::RenderCtx;

/// 参与计算的数值绝对值上限(防 `hi - lo` 溢出成 ∞)。
pub(crate) const VALUE_LIMIT: f64 = 1e300;
/// 单张图表可绘制的标记总数上限(类别数 × 系列数;饼图为点数),超出降级为占位框。
const MAX_MARKS: usize = 20_000;
/// 柱形分类间距(PowerPoint 缺省 `c:gapWidth` = 150%):组宽 = 槽宽 / (1 + 1.5)。
const GAP_RATIO: f64 = 1.5;
/// 估算字宽(em):拉丁字符取 0.55,CJK / 全角取 1.0。
const LATIN_EM: f64 = 0.55;
/// 图例最多行数(超出的条目不画)。
const MAX_LEGEND_ROWS: usize = 3;
/// 降级告警种类。
pub(crate) const CHART_DEGRADED_KIND: &str = "chart-degraded";

/// 图表画不了的原因(调用方据此画占位框 + 告警)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unsupported {
    /// 图类型 v1 不支持(种类名)。
    Kind(String),
    /// 3D 变体。
    ThreeD,
    /// 组合图(多个图类型)。
    Combo,
    /// 无可绘制数据(缺缓存 / 全空 / 饼图无正值)。
    NoData,
    /// 标记数超过 [`MAX_MARKS`]。
    TooManyPoints(usize),
    /// 外框过小,放不下绘图区。
    TooSmall,
}

impl Unsupported {
    /// 告警详情(简体中文,与仓内其它 `Custom` 告警一致)。
    pub(crate) fn detail(&self) -> String {
        match self {
            Unsupported::Kind(k) => format!("图表类型 '{k}' v1 未支持矢量渲染;画占位框"),
            Unsupported::ThreeD => "3D 图表 v1 未支持矢量渲染;画占位框".to_string(),
            Unsupported::Combo => "组合图 v1 未支持矢量渲染;画占位框".to_string(),
            Unsupported::NoData => "图表无可绘制的缓存数据;画占位框".to_string(),
            Unsupported::TooManyPoints(n) => {
                format!("图表数据点过多({n} > {MAX_MARKS});画占位框")
            }
            Unsupported::TooSmall => "图表外框过小;画占位框".to_string(),
        }
    }
}

/// 一段直线(网格线 / 坐标轴)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Seg {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

/// 一个填充矩形(柱 / 条 / 图例色块);`color` 是调色板序号(系列号,饼图为点号)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bar {
    pub rect: Rect,
    pub color: usize,
}

/// 一条折线(遇缺点断开,故一个系列可有多条)。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Polyline {
    pub points: Vec<(f64, f64)>,
    pub color: usize,
}

/// 折线的数据点标记。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Marker {
    pub x: f64,
    pub y: f64,
    pub color: usize,
}

/// 饼图扇形:角度为度,从 12 点方向起、顺时针。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Wedge {
    pub cx: f64,
    pub cy: f64,
    pub r: f64,
    pub start_deg: f64,
    pub sweep_deg: f64,
    pub color: usize,
}

/// 文字框的水平对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LabelAlign {
    Left,
    Center,
    Right,
}

/// 一个单行文字框(垂直居中于 `rect`)。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Label {
    pub rect: Rect,
    pub text: String,
    pub size: f64,
    pub align: LabelAlign,
    pub bold: bool,
}

/// 数值轴刻度:`min..=max`,步长 `step`(`min` / `max` 是 `step` 的整数倍,恒含 0)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AxisScale {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

impl AxisScale {
    /// 各刻度值(最多 101 个)。
    pub(crate) fn ticks(&self) -> Vec<f64> {
        let n = ((self.max - self.min) / self.step).round();
        let n = if n.is_finite() && n >= 1.0 {
            (n as usize).min(100)
        } else {
            1
        };
        (0..=n)
            .map(|i| {
                if i == n {
                    self.max
                } else {
                    self.min + self.step * i as f64
                }
            })
            .collect()
    }

    /// 数值 → `[0, 1]` 比例(0 = `min`)。
    pub(crate) fn frac(&self, v: f64) -> f64 {
        let span = self.max - self.min;
        if span > 0.0 && span.is_finite() {
            ((v - self.min) / span).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// 一张图表的完整几何(页坐标 pt)。
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct ChartGeometry {
    /// 绘图区(柱 / 线 / 饼都在其内)。
    pub plot: Option<Rect>,
    pub gridlines: Vec<Seg>,
    pub axes: Vec<Seg>,
    pub bars: Vec<Bar>,
    pub lines: Vec<Polyline>,
    pub markers: Vec<Marker>,
    pub wedges: Vec<Wedge>,
    /// 图例色块。
    pub swatches: Vec<Bar>,
    pub labels: Vec<Label>,
}

/// 自动"好看"的数值轴:范围恒含 0,步长取 {1, 2, 5} × 10ⁿ、目标约 5 格。
/// 全零 / 非有限输入回退到 `0..1`(步长 0.2)。
pub(crate) fn nice_axis(lo: f64, hi: f64) -> AxisScale {
    const UNIT: AxisScale = AxisScale {
        min: 0.0,
        max: 1.0,
        step: 0.2,
    };
    if !lo.is_finite() || !hi.is_finite() {
        return UNIT;
    }
    let (lo, hi) = (lo.min(0.0), hi.max(0.0));
    let range = hi - lo;
    if range.is_nan() || range <= 0.0 {
        return UNIT;
    }
    let rough = range / 5.0;
    let mag = 10f64.powf(rough.log10().floor());
    let norm = rough / mag;
    let nice = if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    };
    let step = nice * mag;
    let min = (lo / step).floor() * step;
    let max = (hi / step).ceil() * step;
    if step.is_finite() && step > 0.0 && min.is_finite() && max.is_finite() && max > min {
        AxisScale { min, max, step }
    } else {
        // 极端量级(次正规数等)下的兜底:直接用原范围一格。
        AxisScale {
            min: lo,
            max: hi,
            step: range,
        }
    }
}

/// 刻度标签:小数位随步长;量级 ≥ 1e15 或极小时用科学计数;`percent` 追加 `%`;不出现 `-0`。
pub(crate) fn format_tick(v: f64, step: f64, percent: bool) -> String {
    let mag = v.abs().max(step.abs());
    let mut s = if mag >= 1e15 || (step > 0.0 && step < 1e-6) {
        format!("{v:.1e}")
    } else {
        let decimals = if step >= 1.0 || step <= 0.0 {
            0
        } else {
            (-step.log10().floor()).clamp(0.0, 6.0) as usize
        };
        format!("{v:.decimals$}")
    };
    if s.starts_with('-') && s[1..].chars().all(|c| c == '0' || c == '.') {
        s.remove(0);
    }
    if percent {
        s.push('%');
    }
    s
}

/// 系列第 `i` 点的可绘值:缺点 / NaN / ±∞ → `None`,超限截断到 ±[`VALUE_LIMIT`]。
fn value_at(s: &ChartSeries, i: usize) -> Option<f64> {
    s.values
        .get(i)
        .copied()
        .flatten()
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(-VALUE_LIMIT, VALUE_LIMIT))
}

/// 估算单行文字宽(pt)。
fn text_width(text: &str, size: f64) -> f64 {
    text.chars()
        .map(|c| if (c as u32) < 0x2E80 { LATIN_EM } else { 1.0 })
        .sum::<f64>()
        * size
}

/// 截断文字使估算宽不超过 `max_w`(截断处加 `…`);放不下一个字符时为空。
fn fit_text(text: &str, size: f64, max_w: f64) -> String {
    if text_width(text, size) <= max_w {
        return text.to_string();
    }
    let ell = text_width("…", size);
    let mut out = String::new();
    let mut w = 0.0;
    for c in text.chars() {
        let cw = text_width(c.encode_utf8(&mut [0; 4]), size);
        if w + cw + ell > max_w {
            break;
        }
        out.push(c);
        w += cw;
    }
    if out.is_empty() {
        return String::new();
    }
    out.push('…');
    out
}

/// 由数据 + 外框算出图表几何。`frame` 为页坐标 pt。
///
/// # Errors
/// 不支持的图类型 / 3D / 组合 / 无数据 / 点数过多 / 外框过小 → [`Unsupported`]。
pub(crate) fn layout(chart: &Chart, frame: Rect) -> Result<ChartGeometry, Unsupported> {
    match chart.kind {
        ChartKind::Bar | ChartKind::Line | ChartKind::Pie => {}
        ref other => return Err(Unsupported::Kind(other.name().to_string())),
    }
    if chart.combo {
        return Err(Unsupported::Combo);
    }
    if chart.three_d {
        return Err(Unsupported::ThreeD);
    }
    let w = frame.x1 - frame.x0;
    let h = frame.y1 - frame.y0;
    if !(w.is_finite() && h.is_finite()) || w < 40.0 || h < 30.0 {
        return Err(Unsupported::TooSmall);
    }
    // 字号随外框缩放(小图不被文字挤满):标签 6..10pt,标题 ×1.4。
    let size = (w.min(h) / 28.0).clamp(6.0, 10.0);
    let pad = (w.min(h) * 0.03).clamp(2.0, 8.0);
    let mut g = ChartGeometry::default();
    let mut inner = Rect::new(
        frame.x0 + pad,
        frame.y0 + pad,
        frame.x1 - pad,
        frame.y1 - pad,
    );

    // 标题:顶部居中。
    if let Some(title) = chart.title.as_deref().filter(|t| !t.trim().is_empty()) {
        let ts = size * 1.4;
        let band = ts * 1.6;
        let text = fit_text(title.trim(), ts, inner.x1 - inner.x0);
        if !text.is_empty() {
            g.labels.push(Label {
                rect: Rect::new(inner.x0, inner.y0, inner.x1, inner.y0 + band),
                text,
                size: ts,
                align: LabelAlign::Center,
                bold: false,
            });
        }
        inner.y0 += band;
    }

    // 图例条目:柱 / 线为系列(有系列名时),饼为类别。
    let legend: Vec<(String, usize)> = match chart.kind {
        ChartKind::Pie => chart
            .categories
            .iter()
            .enumerate()
            .map(|(i, c)| (c.clone(), i))
            .collect(),
        _ if chart.series.iter().any(|s| s.name.is_some()) => chart
            .series
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let name = s
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("Series {}", i + 1));
                (name, i)
            })
            .collect(),
        _ => Vec::new(),
    };
    if !legend.is_empty() {
        inner.y1 = legend_layout(&mut g, &legend, inner, size);
    }
    if inner.x1 - inner.x0 < 10.0 || inner.y1 - inner.y0 < 10.0 {
        return Err(Unsupported::TooSmall);
    }

    match chart.kind {
        ChartKind::Pie => pie_layout(&mut g, chart, inner)?,
        _ => axis_layout(&mut g, chart, inner, size)?,
    }
    Ok(g)
}

/// 图例:底部居中、按行折排(最多 [`MAX_LEGEND_ROWS`] 行,放不下的条目不画)。
/// 返回绘图可用区域的新底边。
fn legend_layout(
    g: &mut ChartGeometry,
    entries: &[(String, usize)],
    inner: Rect,
    size: f64,
) -> f64 {
    let avail = inner.x1 - inner.x0;
    let sw = size * 0.8;
    let gap = size * 0.4;
    let spacing = size * 1.2;
    let row_h = size * 1.6;
    // 每条:色块 + 间距 + 文字(单条最宽不超过整行)。
    let mut rows: Vec<Vec<(String, usize, f64)>> = vec![Vec::new()];
    let mut row_w = 0.0;
    for (text, color) in entries {
        let text = fit_text(text, size, (avail - sw - gap).max(0.0));
        let ew = sw + gap + text_width(&text, size);
        let need = if row_w > 0.0 { spacing + ew } else { ew };
        if row_w > 0.0 && row_w + need > avail {
            if rows.len() == MAX_LEGEND_ROWS {
                break;
            }
            rows.push(Vec::new());
            row_w = 0.0;
        }
        row_w += if row_w > 0.0 { spacing + ew } else { ew };
        if let Some(row) = rows.last_mut() {
            row.push((text, *color, ew));
        }
    }
    let height = rows.len() as f64 * row_h;
    let top = inner.y1 - height;
    for (ri, row) in rows.iter().enumerate() {
        let total: f64 =
            row.iter().map(|e| e.2).sum::<f64>() + spacing * row.len().saturating_sub(1) as f64;
        let mut x = inner.x0 + (avail - total).max(0.0) / 2.0;
        let cy = top + (ri as f64 + 0.5) * row_h;
        for (text, color, ew) in row {
            g.swatches.push(Bar {
                rect: Rect::new(x, cy - sw / 2.0, x + sw, cy + sw / 2.0),
                color: *color,
            });
            g.labels.push(Label {
                rect: Rect::new(
                    x + sw + gap,
                    cy - row_h / 2.0,
                    x + ew + 1.0,
                    cy + row_h / 2.0,
                ),
                text: text.clone(),
                size,
                align: LabelAlign::Left,
                bold: false,
            });
            x += ew + spacing;
        }
    }
    top - size * 0.4
}

/// 饼图:取首个有正值的系列,按 |值| 比例分扇(0 / 缺点跳过),从 12 点方向顺时针。
fn pie_layout(g: &mut ChartGeometry, chart: &Chart, area: Rect) -> Result<(), Unsupported> {
    let series = chart
        .series
        .iter()
        .find(|s| (0..s.values.len()).any(|i| value_at(s, i).is_some_and(|v| v != 0.0)))
        .ok_or(Unsupported::NoData)?;
    if series.values.len() > MAX_MARKS {
        return Err(Unsupported::TooManyPoints(series.values.len()));
    }
    let vals: Vec<(usize, f64)> = (0..series.values.len())
        .filter_map(|i| value_at(series, i).map(|v| (i, v.abs())))
        .filter(|(_, v)| *v > 0.0)
        .collect();
    let total: f64 = vals.iter().map(|(_, v)| v).sum();
    if !(total.is_finite() && total > 0.0) {
        return Err(Unsupported::NoData);
    }
    let r = (area.x1 - area.x0).min(area.y1 - area.y0) / 2.0 * 0.95;
    let (cx, cy) = ((area.x0 + area.x1) / 2.0, (area.y0 + area.y1) / 2.0);
    g.plot = Some(area);
    let mut start = 0.0;
    for (i, v) in vals {
        let sweep = v / total * 360.0;
        g.wedges.push(Wedge {
            cx,
            cy,
            r,
            start_deg: start,
            sweep_deg: sweep,
            color: i,
        });
        start += sweep;
    }
    Ok(())
}

/// 堆积方式(`c:grouping`)。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stacking {
    None,
    Stacked,
    Percent,
}

/// 一个绘制单元:类别 `cat`、系列 `ser`、值区间 `[base, top]`(簇状时 base = 0)。
struct Item {
    cat: usize,
    ser: usize,
    base: f64,
    top: f64,
}

/// 按堆积方式算出每个 (类别, 系列) 的值区间。柱形的堆积正负分开累加(PowerPoint 同);
/// 折线堆积为逐系列累加。百分比堆积以该类别 |值| 之和为 100%(和为 0 时整类跳过)。
fn stack_items(chart: &Chart, n: usize, stacking: Stacking, bars: bool) -> Vec<Item> {
    let mut items = Vec::new();
    for cat in 0..n {
        let denom: f64 = chart
            .series
            .iter()
            .filter_map(|s| value_at(s, cat))
            .map(f64::abs)
            .sum();
        let scale = match stacking {
            Stacking::Percent if denom > 0.0 && denom.is_finite() => 100.0 / denom,
            Stacking::Percent => continue,
            _ => 1.0,
        };
        let (mut pos, mut neg) = (0.0_f64, 0.0_f64);
        for (ser, s) in chart.series.iter().enumerate() {
            let Some(v) = value_at(s, cat) else {
                continue;
            };
            let v = (v * scale).clamp(-VALUE_LIMIT, VALUE_LIMIT);
            let (base, top) = match stacking {
                Stacking::None => (0.0, v),
                _ if bars && v < 0.0 => {
                    let b = neg;
                    neg = (neg + v).clamp(-VALUE_LIMIT, VALUE_LIMIT);
                    (b, neg)
                }
                _ if bars => {
                    let b = pos;
                    pos = (pos + v).clamp(-VALUE_LIMIT, VALUE_LIMIT);
                    (b, pos)
                }
                _ => {
                    let b = pos;
                    pos = (pos + v).clamp(-VALUE_LIMIT, VALUE_LIMIT);
                    (b, pos)
                }
            };
            items.push(Item {
                cat,
                ser,
                base,
                top,
            });
        }
    }
    items
}

/// 柱形 / 条形 / 折线:数值轴 + 类别轴 + 网格线 + 标签 + 数据标记。
fn axis_layout(
    g: &mut ChartGeometry,
    chart: &Chart,
    area: Rect,
    size: f64,
) -> Result<(), Unsupported> {
    let bars = chart.kind == ChartKind::Bar;
    let horizontal = bars && chart.bar_dir.as_deref() == Some("bar");
    let stacking = match chart.grouping.as_deref() {
        Some("stacked") => Stacking::Stacked,
        Some("percentStacked") => Stacking::Percent,
        _ => Stacking::None,
    };
    let n = chart
        .series
        .iter()
        .map(|s| s.values.len())
        .max()
        .unwrap_or(0)
        .max(chart.categories.len());
    let ns = chart.series.len();
    let marks = n.saturating_mul(ns);
    if marks > MAX_MARKS {
        return Err(Unsupported::TooManyPoints(marks));
    }
    let items = stack_items(chart, n, stacking, bars);
    if n == 0 || items.is_empty() {
        return Err(Unsupported::NoData);
    }
    let (lo, hi) = items.iter().fold((0.0_f64, 0.0_f64), |(lo, hi), it| {
        (lo.min(it.base).min(it.top), hi.max(it.base).max(it.top))
    });
    let scale = nice_axis(lo, hi);
    let percent = stacking == Stacking::Percent;
    let ticks = scale.ticks();
    let tick_text: Vec<String> = ticks
        .iter()
        .map(|t| format_tick(*t, scale.step, percent))
        .collect();
    let cat_text = |i: usize| -> String {
        chart
            .categories
            .get(i)
            .cloned()
            .unwrap_or_else(|| (i + 1).to_string())
    };
    let line_h = size * 1.4;
    let label_gap = size * 0.4;
    let aw = area.x1 - area.x0;

    // 留白:竖向图左侧放数值标签、底部放类别标签;横向条形左侧放类别标签、底部放数值标签。
    let left = if horizontal {
        let widest = (0..n.min(200))
            .map(|i| text_width(&cat_text(i), size))
            .fold(0.0, f64::max);
        widest.min(aw * 0.35) + label_gap
    } else {
        tick_text
            .iter()
            .map(|t| text_width(t, size))
            .fold(0.0, f64::max)
            .min(aw * 0.35)
            + label_gap
    };
    let plot = Rect::new(
        area.x0 + left,
        area.y0 + line_h * 0.5,
        area.x1 - size * 0.5,
        area.y1 - line_h - label_gap,
    );
    let (pw, ph) = (plot.x1 - plot.x0, plot.y1 - plot.y0);
    if pw < 10.0 || ph < 10.0 {
        return Err(Unsupported::TooSmall);
    }
    g.plot = Some(plot);

    // 数值 → 页坐标(竖向:y 自底向上;横向:x 自左向右)。
    let val_pos = |v: f64| -> f64 {
        if horizontal {
            plot.x0 + scale.frac(v) * pw
        } else {
            plot.y1 - scale.frac(v) * ph
        }
    };
    // 类别槽 [a, b](竖向:自左向右;横向:首类别在底部,PowerPoint 同)。
    let slot_len = if horizontal { ph } else { pw } / n as f64;
    let slot = |i: usize| -> (f64, f64) {
        if horizontal {
            let b = plot.y1 - i as f64 * slot_len;
            (b - slot_len, b)
        } else {
            let a = plot.x0 + i as f64 * slot_len;
            (a, a + slot_len)
        }
    };

    // 网格线 + 数值标签。
    for (t, text) in ticks.iter().zip(&tick_text) {
        let p = val_pos(*t);
        if horizontal {
            g.gridlines.push(Seg {
                x1: p,
                y1: plot.y0,
                x2: p,
                y2: plot.y1,
            });
            let tw = text_width(text, size) + size;
            g.labels.push(Label {
                rect: Rect::new(
                    p - tw / 2.0,
                    plot.y1 + label_gap,
                    p + tw / 2.0,
                    plot.y1 + label_gap + line_h,
                ),
                text: text.clone(),
                size,
                align: LabelAlign::Center,
                bold: false,
            });
        } else {
            g.gridlines.push(Seg {
                x1: plot.x0,
                y1: p,
                x2: plot.x1,
                y2: p,
            });
            g.labels.push(Label {
                rect: Rect::new(
                    area.x0,
                    p - line_h / 2.0,
                    plot.x0 - label_gap,
                    p + line_h / 2.0,
                ),
                text: text.clone(),
                size,
                align: LabelAlign::Right,
                bold: false,
            });
        }
    }
    // 类别轴线画在数值 0 处(范围恒含 0)。
    let zero = val_pos(0.0);
    g.axes.push(if horizontal {
        Seg {
            x1: zero,
            y1: plot.y0,
            x2: zero,
            y2: plot.y1,
        }
    } else {
        Seg {
            x1: plot.x0,
            y1: zero,
            x2: plot.x1,
            y2: zero,
        }
    });

    // 类别标签(过密时每 k 个取一个)。
    let label_extent = if horizontal {
        line_h
    } else {
        (0..n.min(200))
            .map(|i| text_width(&cat_text(i), size))
            .fold(0.0, f64::max)
            .min(aw * 0.25)
            + size
    };
    let every = (label_extent / slot_len).ceil().max(1.0);
    let every = if every.is_finite() { every as usize } else { n };
    for i in (0..n).step_by(every.max(1)) {
        let (a, b) = slot(i);
        let span = slot_len * every as f64;
        let rect = if horizontal {
            let c = (a + b) / 2.0;
            Rect::new(
                area.x0,
                c - line_h / 2.0,
                plot.x0 - label_gap,
                c + line_h / 2.0,
            )
        } else {
            let c = (a + b) / 2.0;
            Rect::new(
                c - span / 2.0,
                plot.y1 + label_gap,
                c + span / 2.0,
                plot.y1 + label_gap + line_h,
            )
        };
        let text = fit_text(&cat_text(i), size, rect.x1 - rect.x0);
        if !text.is_empty() {
            g.labels.push(Label {
                rect,
                text,
                size,
                align: if horizontal {
                    LabelAlign::Right
                } else {
                    LabelAlign::Center
                },
                bold: false,
            });
        }
    }

    if bars {
        // 组宽 = 槽长 / (1 + gap);簇状时每系列占组宽 / 系列数,堆积时整组一根。
        let group = slot_len / (1.0 + GAP_RATIO);
        let per = if stacking == Stacking::None {
            group / ns.max(1) as f64
        } else {
            group
        };
        for it in &items {
            let (a, b) = slot(it.cat);
            let offset = if stacking == Stacking::None {
                it.ser as f64 * per
            } else {
                0.0
            };
            let (p0, p1) = (val_pos(it.base), val_pos(it.top));
            let rect = if horizontal {
                // 横向:组内首系列在下(与首类别在下一致)。
                let y1 = b - (slot_len - group) / 2.0 - offset;
                Rect::new(p0.min(p1), y1 - per, p0.max(p1), y1)
            } else {
                let x0 = a + (slot_len - group) / 2.0 + offset;
                Rect::new(x0, p0.min(p1), x0 + per, p0.max(p1))
            };
            if rect.x1 - rect.x0 > 0.0 && rect.y1 - rect.y0 > 0.0 {
                g.bars.push(Bar {
                    rect,
                    color: it.ser,
                });
            }
        }
    } else {
        // 折线:点在槽中心;同一系列遇缺点断开。
        for ser in 0..ns {
            let mut current: Vec<(f64, f64)> = Vec::new();
            let mut prev_cat: Option<usize> = None;
            for it in items.iter().filter(|it| it.ser == ser) {
                let (a, b) = slot(it.cat);
                let pt = ((a + b) / 2.0, val_pos(it.top));
                if prev_cat.is_some_and(|p| p + 1 != it.cat) && !current.is_empty() {
                    push_polyline(g, std::mem::take(&mut current), ser);
                }
                current.push(pt);
                g.markers.push(Marker {
                    x: pt.0,
                    y: pt.1,
                    color: ser,
                });
                prev_cat = Some(it.cat);
            }
            push_polyline(g, current, ser);
        }
    }
    Ok(())
}

/// 至少两点才成线(孤立点只留标记)。
fn push_polyline(g: &mut ChartGeometry, points: Vec<(f64, f64)>, color: usize) {
    if points.len() >= 2 {
        g.lines.push(Polyline { points, color });
    }
}

/// 扇形 → 闭合路径:圆心 → 弧起点 → 每段 ≤ 90° 的三次贝塞尔逼近圆弧 → 回圆心。
/// 整圆(≥ 359.999°)不连圆心,避免描边出一条半径。
pub(crate) fn wedge_segs(w: &Wedge) -> Vec<PathSeg> {
    let full = w.sweep_deg >= 359.999;
    let sweep = w.sweep_deg.clamp(0.0, 360.0);
    let pt = |deg: f64| -> (f64, f64) {
        let a = deg.to_radians();
        (w.cx + w.r * a.sin(), w.cy - w.r * a.cos())
    };
    let n = (sweep / 90.0).ceil().max(1.0) as usize;
    let step = sweep / n as f64;
    let k = 4.0 / 3.0 * (step.to_radians() / 4.0).tan() * w.r;
    let (sx, sy) = pt(w.start_deg);
    let mut segs = Vec::with_capacity(n + 3);
    if full {
        segs.push(PathSeg::MoveTo { x: sx, y: sy });
    } else {
        segs.push(PathSeg::MoveTo { x: w.cx, y: w.cy });
        segs.push(PathSeg::LineTo { x: sx, y: sy });
    }
    for i in 0..n {
        let a0 = w.start_deg + step * i as f64;
        let a1 = a0 + step;
        let (x0, y0) = pt(a0);
        let (x1, y1) = pt(a1);
        // 顺时针(y 向下)切向 = (cos a, sin a)。
        let (r0, r1) = (a0.to_radians(), a1.to_radians());
        segs.push(PathSeg::CurveTo {
            x1: x0 + k * r0.cos(),
            y1: y0 + k * r0.sin(),
            x2: x1 - k * r1.cos(),
            y2: y1 - k * r1.sin(),
            x: x1,
            y: y1,
        });
    }
    segs.push(PathSeg::Close);
    segs
}

/// 网格线色(浅灰 D9D9D9)。
const GRID: Rgb = Rgb {
    r: 0.851,
    g: 0.851,
    b: 0.851,
};
/// 坐标轴线色(BFBFBF)。
const AXIS: Rgb = Rgb {
    r: 0.749,
    g: 0.749,
    b: 0.749,
};
/// 标签文字色(595959,PowerPoint 缺省图表文字)。
const LABEL: Rgb = Rgb {
    r: 0.349,
    g: 0.349,
    b: 0.349,
};
/// 折线线宽(PowerPoint 缺省 2.25pt)。
const LINE_WIDTH_PT: f64 = 2.25;

/// 调色板第 `i` 色:accent1..6 循环。
fn palette(accents: &[[u8; 3]; 6], i: usize) -> Rgb {
    let c = accents[i % 6];
    Rgb::new(
        f64::from(c[0]) / 255.0,
        f64::from(c[1]) / 255.0,
        f64::from(c[2]) / 255.0,
    )
}

/// 把图表画成矢量 op;画不了时记 `chart-degraded` 告警并返回 `false`(调用方画占位框)。
pub(crate) fn chart_ops(
    ts: &mut Typesetter,
    ctx: &mut RenderCtx<'_>,
    chart: &Chart,
    frame: Rect,
    ops: &mut Vec<Op>,
) -> bool {
    let g = match layout(chart, frame) {
        Ok(g) => g,
        Err(why) => {
            ctx.warnings.push(ExportWarning::Custom {
                kind: CHART_DEGRADED_KIND.to_string(),
                detail: why.detail(),
            });
            return false;
        }
    };
    let accents = ctx.accents;
    let line = |s: &Seg, color: Rgb| Op::Line {
        x1: s.x1,
        y1: s.y1,
        x2: s.x2,
        y2: s.y2,
        color,
        width: 0.75,
    };
    ops.extend(g.gridlines.iter().map(|s| line(s, GRID)));
    let fill_rect = |b: &Bar| Op::FillRect {
        x: b.rect.x0,
        y: b.rect.y0,
        w: b.rect.x1 - b.rect.x0,
        h: b.rect.y1 - b.rect.y0,
        color: palette(&accents, b.color),
    };
    ops.extend(g.bars.iter().map(fill_rect));
    for w in &g.wedges {
        let mut sep = Stroke::new(Rgb::new(1.0, 1.0, 1.0), 1.0);
        sep.join = LineJoin::Round;
        ops.push(Op::Path {
            segs: wedge_segs(w),
            fill: Some(Fill::new(palette(&accents, w.color))),
            stroke: Some(sep),
        });
    }
    ops.extend(g.axes.iter().map(|s| line(s, AXIS)));
    for pl in &g.lines {
        let mut segs = Vec::with_capacity(pl.points.len());
        for (i, &(x, y)) in pl.points.iter().enumerate() {
            segs.push(if i == 0 {
                PathSeg::MoveTo { x, y }
            } else {
                PathSeg::LineTo { x, y }
            });
        }
        let mut stroke = Stroke::new(palette(&accents, pl.color), LINE_WIDTH_PT);
        stroke.cap = LineCap::Round;
        stroke.join = LineJoin::Round;
        ops.push(Op::Path {
            segs,
            fill: None,
            stroke: Some(stroke),
        });
    }
    for m in &g.markers {
        ops.push(Op::FillCircle {
            cx: m.x,
            cy: m.y,
            r: LINE_WIDTH_PT * 1.2,
            color: palette(&accents, m.color),
        });
    }
    ops.extend(g.swatches.iter().map(fill_rect));
    for l in &g.labels {
        let mut style = RunStyle::new(crate::text::DEFAULT_LATIN, l.size);
        style.color = LABEL;
        style.bold = l.bold;
        let mut props = ParaProps::new();
        props.align = match l.align {
            LabelAlign::Left => Align::Left,
            LabelAlign::Center => Align::Center,
            LabelAlign::Right => Align::Right,
        };
        let mut spec = TextBoxSpec::new(
            l.rect,
            vec![Block::Paragraph(
                props,
                vec![Run::new(l.text.clone(), style)],
            )],
        );
        spec.v_anchor = VAnchor::Middle;
        spec.wrap = false;
        ops.extend(ts.layout_text_box(&spec));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(name: Option<&str>, values: &[Option<f64>]) -> ChartSeries {
        ChartSeries {
            name: name.map(str::to_string),
            values: values.to_vec(),
            format_code: None,
        }
    }

    fn chart(kind: ChartKind, cats: &[&str], series: Vec<ChartSeries>) -> Chart {
        Chart {
            kind,
            title: None,
            categories: cats.iter().map(|s| s.to_string()).collect(),
            series,
            bar_dir: None,
            grouping: None,
            three_d: false,
            combo: false,
            warnings: Vec::new(),
        }
    }

    fn frame() -> Rect {
        Rect::new(100.0, 50.0, 500.0, 350.0)
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6 * (1.0 + a.abs().max(b.abs()))
    }

    fn assert_finite(g: &ChartGeometry) {
        let ok = |v: f64| v.is_finite();
        for b in g.bars.iter().chain(&g.swatches) {
            assert!(
                ok(b.rect.x0) && ok(b.rect.y0) && ok(b.rect.x1) && ok(b.rect.y1),
                "{b:?}"
            );
        }
        for s in g.gridlines.iter().chain(&g.axes) {
            assert!(ok(s.x1) && ok(s.y1) && ok(s.x2) && ok(s.y2), "{s:?}");
        }
        for l in &g.lines {
            assert!(l.points.iter().all(|(x, y)| ok(*x) && ok(*y)));
        }
        for w in &g.wedges {
            assert!(ok(w.start_deg) && ok(w.sweep_deg) && ok(w.r));
        }
        for l in &g.labels {
            assert!(ok(l.rect.x0) && ok(l.rect.y0) && ok(l.rect.x1) && ok(l.rect.y1));
        }
    }

    // ---- 刻度 -----------------------------------------------------------------

    #[test]
    fn nice_axis_positive_range() {
        let a = nice_axis(3.0, 87.0);
        assert_eq!((a.min, a.max, a.step), (0.0, 100.0, 20.0));
        assert_eq!(a.ticks(), vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0]);
    }

    #[test]
    fn nice_axis_negative_and_cross_zero() {
        let a = nice_axis(-37.0, -2.0);
        assert_eq!((a.min, a.max, a.step), (-40.0, 0.0, 10.0), "全负:上界为 0");
        let a = nice_axis(-12.0, 33.0);
        assert_eq!((a.min, a.max, a.step), (-20.0, 40.0, 10.0), "跨零");
        assert!(a.ticks().contains(&0.0), "0 恒为刻度");
    }

    #[test]
    fn nice_axis_all_zero_single_value_and_nan() {
        for (lo, hi) in [
            (0.0, 0.0),
            (f64::NAN, 1.0),
            (0.0, f64::INFINITY),
            (f64::NAN, f64::NAN),
        ] {
            let a = nice_axis(lo, hi);
            assert_eq!((a.min, a.max, a.step), (0.0, 1.0, 0.2), "{lo}..{hi}");
        }
        let a = nice_axis(7.0, 7.0);
        assert_eq!((a.min, a.max, a.step), (0.0, 8.0, 2.0), "单值:0 基线");
        let a = nice_axis(0.003, 0.003);
        assert!(
            a.min == 0.0 && approx(a.max, 0.003) && approx(a.step, 0.001),
            "{a:?}"
        );
    }

    #[test]
    fn nice_axis_extreme_magnitudes_stay_finite() {
        for (lo, hi) in [
            (-VALUE_LIMIT, VALUE_LIMIT),
            (0.0, VALUE_LIMIT),
            (0.0, 1e-310),
            (-f64::MIN_POSITIVE, 0.0),
        ] {
            let a = nice_axis(lo, hi);
            assert!(
                a.min.is_finite() && a.max.is_finite() && a.step.is_finite(),
                "{a:?}"
            );
            assert!(a.max > a.min && a.step > 0.0, "{a:?}");
            assert!(a.min <= lo.min(0.0) && a.max >= hi.max(0.0), "{a:?}");
            let t = a.ticks();
            assert!(t.len() >= 2 && t.len() <= 101 && t.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn tick_labels_follow_step() {
        assert_eq!(format_tick(20.0, 20.0, false), "20");
        assert_eq!(format_tick(0.4, 0.2, false), "0.4");
        assert_eq!(format_tick(0.15, 0.05, false), "0.15");
        assert_eq!(format_tick(-0.0, 1.0, false), "0");
        assert_eq!(format_tick(60.0, 20.0, true), "60%");
        assert_eq!(format_tick(2e20, 5e19, false), "2.0e20");
    }

    // ---- 柱形 / 条形 ------------------------------------------------------------

    #[test]
    fn clustered_columns_are_proportional_and_ordered() {
        let c = chart(
            ChartKind::Bar,
            &["A", "B"],
            vec![
                series(Some("S1"), &[Some(10.0), Some(20.0)]),
                series(Some("S2"), &[Some(5.0), Some(40.0)]),
            ],
        );
        let g = layout(&c, frame()).expect("bar layout");
        assert_eq!(g.bars.len(), 4);
        let plot = g.plot.expect("plot");
        let zero = plot.y1; // 全正:0 在底边
        let h = |i: usize| g.bars[i].rect.y1 - g.bars[i].rect.y0;
        // 类别 A:S1, S2;类别 B:S1, S2(文档序)。轴上界 40。
        assert!(approx(h(0) / h(3), 10.0 / 40.0));
        assert!(approx(h(1) / h(3), 5.0 / 40.0));
        assert!(approx(h(2) / h(3), 20.0 / 40.0));
        assert!(approx(h(3), plot.y1 - plot.y0), "最大值顶到轴上界");
        for b in &g.bars {
            assert!(approx(b.rect.y1, zero), "柱底在 0 基线");
        }
        // 同类别内 S1 在 S2 左侧且等宽、不重叠;类别 A 整组在 B 左侧。
        assert!(g.bars[0].rect.x1 <= g.bars[1].rect.x0 + 1e-9);
        assert!(approx(
            g.bars[0].rect.x1 - g.bars[0].rect.x0,
            g.bars[1].rect.x1 - g.bars[1].rect.x0
        ));
        assert!(g.bars[1].rect.x1 < g.bars[2].rect.x0);
        assert_eq!(
            g.bars.iter().map(|b| b.color).collect::<Vec<_>>(),
            vec![0, 1, 0, 1],
            "按系列配色"
        );
        // 图例两条,类别标签两个。
        assert_eq!(g.swatches.len(), 2);
        assert!(g.labels.iter().any(|l| l.text == "A") && g.labels.iter().any(|l| l.text == "S2"));
    }

    #[test]
    fn negative_columns_hang_below_zero_baseline() {
        let c = chart(
            ChartKind::Bar,
            &["A", "B"],
            vec![series(None, &[Some(-10.0), Some(30.0)])],
        );
        let g = layout(&c, frame()).expect("layout");
        let axis = g.axes[0];
        assert!(approx(axis.y1, axis.y2));
        let zero = axis.y1;
        assert!(approx(g.bars[0].rect.y0, zero), "负柱顶在 0 基线");
        assert!(g.bars[0].rect.y1 > zero, "负柱向下");
        assert!(approx(g.bars[1].rect.y1, zero), "正柱底在 0 基线");
        let (hn, hp) = (
            g.bars[0].rect.y1 - g.bars[0].rect.y0,
            g.bars[1].rect.y1 - g.bars[1].rect.y0,
        );
        assert!(approx(hn / hp, 10.0 / 30.0));
        assert!(g.swatches.is_empty(), "无系列名不画图例");
    }

    #[test]
    fn stacked_columns_accumulate_and_percent_fills_axis() {
        let mut c = chart(
            ChartKind::Bar,
            &["A"],
            vec![
                series(Some("S1"), &[Some(10.0)]),
                series(Some("S2"), &[Some(30.0)]),
                series(Some("S3"), &[Some(-5.0)]),
            ],
        );
        c.grouping = Some("stacked".into());
        let g = layout(&c, frame()).expect("stacked");
        assert_eq!(g.bars.len(), 3);
        let (b0, b1, b2) = (g.bars[0].rect, g.bars[1].rect, g.bars[2].rect);
        assert!(approx(b1.y1, b0.y0), "S2 叠在 S1 之上");
        assert!(approx((b1.y1 - b1.y0) / (b0.y1 - b0.y0), 3.0));
        assert!(approx(b2.y0, b0.y1), "负值从 0 基线向下另起一摞");
        assert!(approx(b0.x0, b1.x0) && approx(b0.x1, b1.x1), "堆积同宽同位");

        c.grouping = Some("percentStacked".into());
        let g = layout(&c, frame()).expect("percent");
        let total: f64 = g.bars.iter().map(|b| b.rect.y1 - b.rect.y0).sum();
        let plot = g.plot.expect("plot");
        let scale = nice_axis(-5.0 / 45.0 * 100.0, 40.0 / 45.0 * 100.0);
        let px_per_unit = (plot.y1 - plot.y0) / (scale.max - scale.min);
        assert!(approx(total, 100.0 * px_per_unit), "三段合计 = 100%");
        assert!(g.labels.iter().any(|l| l.text.ends_with('%')), "百分比刻度");
    }

    #[test]
    fn horizontal_bars_put_first_category_at_bottom() {
        let mut c = chart(
            ChartKind::Bar,
            &["First", "Second"],
            vec![series(None, &[Some(1.0), Some(2.0)])],
        );
        c.bar_dir = Some("bar".into());
        let g = layout(&c, frame()).expect("bar");
        let plot = g.plot.expect("plot");
        let (b0, b1) = (g.bars[0].rect, g.bars[1].rect);
        assert!(b0.y0 > b1.y0, "首类别在下");
        assert!(
            approx(b0.x0, plot.x0) && approx(b1.x0, plot.x0),
            "条从左侧 0 基线起"
        );
        assert!(approx((b1.x1 - b1.x0) / (b0.x1 - b0.x0), 2.0));
        assert!(approx(b0.y1 - b0.y0, b1.y1 - b1.y0));
    }

    // ---- 折线 -------------------------------------------------------------------

    #[test]
    fn line_points_sit_at_slot_centers_and_break_on_gaps() {
        let c = chart(
            ChartKind::Line,
            &["a", "b", "c", "d"],
            vec![series(Some("L"), &[Some(0.0), Some(10.0), None, Some(5.0)])],
        );
        let g = layout(&c, frame()).expect("line");
        let plot = g.plot.expect("plot");
        let slot = (plot.x1 - plot.x0) / 4.0;
        assert_eq!(g.lines.len(), 1, "缺点断开,孤立点不成线");
        let pts = &g.lines[0].points;
        assert_eq!(pts.len(), 2);
        assert!(approx(pts[0].0, plot.x0 + slot * 0.5));
        assert!(approx(pts[1].0, plot.x0 + slot * 1.5));
        assert!(approx(pts[0].1, plot.y1), "0 在底边");
        assert!(approx(pts[1].1, plot.y0), "10 = 轴上界");
        assert_eq!(g.markers.len(), 3, "孤立点仍有标记");
        assert!(approx(g.markers[2].x, plot.x0 + slot * 3.5));
        assert!(approx(g.markers[2].y, plot.y1 - (plot.y1 - plot.y0) * 0.5));
    }

    #[test]
    fn mismatched_series_lengths_use_longest_and_categories() {
        let c = chart(
            ChartKind::Line,
            &["a", "b"],
            vec![
                series(Some("short"), &[Some(1.0)]),
                series(Some("long"), &[Some(1.0), Some(2.0), Some(3.0)]),
            ],
        );
        let g = layout(&c, frame()).expect("line");
        assert_eq!(g.markers.len(), 4);
        assert!(g.labels.iter().any(|l| l.text == "3"), "无类别名的槽用序号");
        assert_finite(&g);
    }

    // ---- 饼图 -------------------------------------------------------------------

    #[test]
    fn pie_angles_sum_to_360_and_are_proportional() {
        let c = chart(
            ChartKind::Pie,
            &["x", "y", "z", "w"],
            vec![series(
                Some("P"),
                &[Some(1.0), Some(3.0), Some(0.0), Some(-4.0)],
            )],
        );
        let g = layout(&c, frame()).expect("pie");
        assert_eq!(g.wedges.len(), 3, "0 值跳过");
        let total: f64 = g.wedges.iter().map(|w| w.sweep_deg).sum();
        assert!(approx(total, 360.0));
        assert!(approx(g.wedges[0].sweep_deg, 45.0));
        assert!(approx(g.wedges[1].sweep_deg, 135.0));
        assert!(approx(g.wedges[2].sweep_deg, 180.0), "负值取绝对值");
        assert!(approx(g.wedges[0].start_deg, 0.0));
        assert!(approx(g.wedges[1].start_deg, 45.0));
        assert_eq!(
            g.wedges.iter().map(|w| w.color).collect::<Vec<_>>(),
            vec![0, 1, 3]
        );
        assert_eq!(g.swatches.len(), 4, "图例按类别");
    }

    #[test]
    fn wedge_path_starts_at_center_and_ends_on_arc() {
        let w = Wedge {
            cx: 0.0,
            cy: 0.0,
            r: 10.0,
            start_deg: 0.0,
            sweep_deg: 180.0,
            color: 0,
        };
        let segs = wedge_segs(&w);
        assert_eq!(segs[0], PathSeg::MoveTo { x: 0.0, y: 0.0 });
        assert!(
            matches!(segs[1], PathSeg::LineTo { x, y } if approx(x, 0.0) && approx(y, -10.0)),
            "12 点方向"
        );
        assert_eq!(
            segs.iter()
                .filter(|s| matches!(s, PathSeg::CurveTo { .. }))
                .count(),
            2
        );
        assert!(
            matches!(segs[3], PathSeg::CurveTo { x, y, .. } if x.abs() < 1e-9 && approx(y, 10.0)),
            "顺时针到 6 点"
        );
        let full = wedge_segs(&Wedge {
            sweep_deg: 360.0,
            ..w
        });
        assert!(
            !full.iter().any(|s| matches!(s, PathSeg::LineTo { .. })),
            "整圆不连圆心"
        );
    }

    // ---- 降级 / 健壮性 ------------------------------------------------------------

    #[test]
    fn unsupported_kinds_3d_combo_and_empty_data_degrade() {
        let one = || vec![series(None, &[Some(1.0)])];
        for kind in [
            ChartKind::Area,
            ChartKind::Scatter,
            ChartKind::Radar,
            ChartKind::Doughnut,
        ] {
            let name = kind.name().to_string();
            assert_eq!(
                layout(&chart(kind, &[], one()), frame()),
                Err(Unsupported::Kind(name))
            );
        }
        let mut c = chart(ChartKind::Bar, &[], one());
        c.three_d = true;
        assert_eq!(layout(&c, frame()), Err(Unsupported::ThreeD));
        let mut c = chart(ChartKind::Bar, &[], one());
        c.combo = true;
        assert_eq!(layout(&c, frame()), Err(Unsupported::Combo));
        for kind in [ChartKind::Bar, ChartKind::Line, ChartKind::Pie] {
            assert_eq!(
                layout(&chart(kind.clone(), &[], vec![]), frame()),
                Err(Unsupported::NoData)
            );
            let empty = vec![
                series(Some("e"), &[]),
                series(None, &[None, Some(f64::NAN)]),
            ];
            assert_eq!(
                layout(&chart(kind, &["a"], empty), frame()),
                Err(Unsupported::NoData)
            );
        }
        let zeros = vec![series(None, &[Some(0.0), Some(0.0)])];
        assert_eq!(
            layout(&chart(ChartKind::Pie, &[], zeros), frame()),
            Err(Unsupported::NoData)
        );
        let tiny = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(
            layout(&chart(ChartKind::Bar, &[], one()), tiny),
            Err(Unsupported::TooSmall)
        );
    }

    #[test]
    fn all_zero_columns_draw_axis_without_bars() {
        let c = chart(
            ChartKind::Bar,
            &["a", "b"],
            vec![series(None, &[Some(0.0), Some(0.0)])],
        );
        let g = layout(&c, frame()).expect("all zero");
        assert!(g.bars.is_empty(), "零高柱不画");
        assert_eq!(g.gridlines.len(), 6, "0..1 步长 0.2");
        assert_finite(&g);
    }

    #[test]
    fn nan_infinite_and_huge_values_never_panic() {
        let vals = [
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(f64::NEG_INFINITY),
            Some(f64::MAX),
            Some(-f64::MAX),
            Some(1.0),
            None,
        ];
        for kind in [ChartKind::Bar, ChartKind::Line, ChartKind::Pie] {
            for grouping in [None, Some("stacked"), Some("percentStacked")] {
                let mut c = chart(
                    kind.clone(),
                    &["a"],
                    vec![series(Some("s"), &vals), series(Some("t"), &vals)],
                );
                c.grouping = grouping.map(str::to_string);
                let g = layout(&c, frame()).expect("finite values remain");
                assert_finite(&g);
            }
        }
    }

    #[test]
    fn many_categories_thin_labels_and_too_many_points_degrade() {
        let vals: Vec<Option<f64>> = (0..500).map(|i| Some(f64::from(i))).collect();
        let cats: Vec<String> = (0..500).map(|i| format!("Category {i}")).collect();
        let cat_refs: Vec<&str> = cats.iter().map(String::as_str).collect();
        let c = chart(ChartKind::Bar, &cat_refs, vec![series(None, &vals)]);
        let g = layout(&c, frame()).expect("many");
        assert_eq!(g.bars.len(), 499, "值 0 的柱不画");
        let cat_labels = g
            .labels
            .iter()
            .filter(|l| l.text.starts_with("Cat"))
            .count();
        assert!(
            cat_labels > 0 && cat_labels < 50,
            "类别标签抽稀:{cat_labels}"
        );
        let big: Vec<Option<f64>> = vec![Some(1.0); MAX_MARKS + 1];
        let c = chart(ChartKind::Line, &[], vec![series(None, &big)]);
        assert_eq!(
            layout(&c, frame()),
            Err(Unsupported::TooManyPoints(MAX_MARKS + 1))
        );
    }

    #[test]
    fn title_and_layout_are_deterministic() {
        let mut c = chart(
            ChartKind::Bar,
            &["A", "B", "C"],
            vec![series(Some("S"), &[Some(1.0), Some(2.0), Some(3.0)])],
        );
        c.title = Some("Revenue".into());
        let g1 = layout(&c, frame()).expect("a");
        let g2 = layout(&c, frame()).expect("b");
        assert_eq!(g1, g2);
        let title = g1
            .labels
            .iter()
            .find(|l| l.text == "Revenue")
            .expect("title");
        assert_eq!(title.align, LabelAlign::Center);
        assert!(
            title.rect.y1 <= g1.plot.expect("plot").y0,
            "标题在绘图区之上"
        );
    }
}
