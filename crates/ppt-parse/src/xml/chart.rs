//! 解析图表部件 `ppt/charts/chartN.xml`(`c:chartSpace`)-> [`Chart`]。
//!
//! 只读部件内的**缓存**:标题(`c:title > c:tx` 的 `c:rich` 富文本或 `c:strRef > c:strCache`)、
//! `c:plotArea` 下各图类型的系列(`c:ser`):系列名 `c:tx`、类别 `c:cat`(散点 / 气泡为 `c:xVal`)、
//! 值 `c:val`(散点 / 气泡为 `c:yVal`)。数据源支持 `strRef` / `numRef`(取其 cache)、
//! `strLit` / `numLit`、`multiLvlStrRef`(取首层)。`pt@idx` 可能稀疏,按 `ptCount` 补空。
//! **不**解析外部工作簿;缺缓存的系列记为空并写入 [`Chart::warnings`]。
//!
//! 容错:未知元素跳过、畸形数字 → 缺点、绝不 panic;点数 / 系列数设上限防放大攻击。

use ppt_core::color::ColorSpec;
use ppt_core::model::{Chart, ChartKind, ChartSeries, DataLabels};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::slide::parse_ln;
use super::text_style::parse_solid_fill;
use super::{attr_of, local_name, read_text, skip_element};

/// 单个缓存的最大点数(超出的 `idx` 丢弃;`ptCount` 截到此值)。
const MAX_POINTS: usize = 1 << 20;
/// 整张图表物化的点数总预算(所有系列的类别 + 值)。
const MAX_TOTAL_POINTS: usize = 1 << 22;
/// 单张图表的最大系列数。
const MAX_SERIES: usize = 1024;

/// 解析状态:告警 + 剩余点数预算。
struct State {
    warnings: Vec<String>,
    budget: usize,
}

impl State {
    fn warn(&mut self, msg: String) {
        if !self.warnings.contains(&msg) {
            self.warnings.push(msg);
        }
    }
}

/// 一个数据源(`c:cat` / `c:val` / `c:xVal` / `c:yVal`)的缓存内容。
#[derive(Debug, Default)]
struct Cache {
    /// 点文本(`c:pt > c:v`),按 idx 落位;稀疏处为 `None`。
    points: Vec<Option<String>>,
    format_code: Option<String>,
    /// 是否见到了缓存 / 字面量(`numRef` 只有 `c:f` 时为 false)。
    found: bool,
}

/// 一个 `c:ser` 的原始解析结果。
#[derive(Debug, Default)]
struct RawSeries {
    name: Option<String>,
    cat: Option<Cache>,
    val: Option<Cache>,
    /// `c:spPr > a:solidFill`。
    fill: Option<ColorSpec>,
    /// `c:spPr > a:ln > a:solidFill`。
    line: Option<ColorSpec>,
    point_colors: Vec<(usize, ColorSpec)>,
    /// 系列级 `c:dLbls`(`c:delete` 记全 false,以便覆盖类型级)。
    labels: Option<DataLabels>,
}

/// 解析一份图表部件 XML。找不到任何图类型时 `kind` 记为 `Other("")`。
pub fn parse(xml: &str) -> Chart {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut st = State {
        warnings: Vec::new(),
        budget: MAX_TOTAL_POINTS,
    };
    let mut chart = Chart {
        kind: ChartKind::Other(String::new()),
        title: None,
        categories: Vec::new(),
        series: Vec::new(),
        bar_dir: None,
        grouping: None,
        three_d: false,
        combo: false,
        of_pie: false,
        warnings: Vec::new(),
    };
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == b"chart" {
                    parse_chart_el(&mut reader, &mut chart, &mut st);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    chart.warnings = st.warnings;
    chart
}

/// `c:chart`:`c:title` + `c:autoTitleDeleted` + `c:plotArea`。已消费起始标签。
fn parse_chart_el<R: std::io::BufRead>(reader: &mut Reader<R>, chart: &mut Chart, st: &mut State) {
    let mut title: Option<Option<String>> = None; // Some(None) = 有 c:title 但无 c:tx(自动标题)
    let mut auto_deleted = false;
    let mut kinds: Vec<(ChartKind, PlotInfo)> = Vec::new();
    let mut raw: Vec<(ChartKind, RawSeries)> = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"title" => title = Some(parse_title(reader)),
                    b"plotArea" => parse_plot_area(reader, &mut kinds, &mut raw, st),
                    b"autoTitleDeleted" => {
                        auto_deleted = val_true(&e);
                        skip_element(reader, &name);
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => match local_name(e.name().as_ref()) {
                b"title" => title = Some(None),
                b"autoTitleDeleted" => auto_deleted = val_true(&e),
                _ => {}
            },
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    chart.combo = kinds.len() > 1;
    if let Some((k, info)) = kinds.into_iter().next() {
        chart.kind = k;
        chart.bar_dir = info.bar_dir;
        chart.grouping = info.grouping;
        chart.three_d = info.three_d;
        chart.of_pie = info.of_pie;
    }
    for (kind, rs) in raw {
        let label = rs.name.clone().unwrap_or_else(|| "(unnamed)".to_string());
        let (cat_tag, val_tag) = if matches!(kind, ChartKind::Scatter | ChartKind::Bubble) {
            ("xVal", "yVal")
        } else {
            ("cat", "val")
        };
        if let Some(cat) = rs.cat {
            if !cat.found {
                st.warn(format!(
                    "chart series '{label}': {cat_tag} cache missing (external workbook not read)"
                ));
            } else if chart.categories.is_empty() {
                chart.categories = cat
                    .points
                    .into_iter()
                    .map(Option::unwrap_or_default)
                    .collect();
            }
        }
        let (values, format_code) = match rs.val {
            Some(v) if v.found => (
                v.points
                    .iter()
                    .map(|p| p.as_deref().and_then(|s| s.trim().parse::<f64>().ok()))
                    .collect(),
                v.format_code,
            ),
            _ => {
                st.warn(format!(
                    "chart series '{label}': {val_tag} cache missing (external workbook not read)"
                ));
                (Vec::new(), None)
            }
        };
        chart.series.push(ChartSeries {
            name: rs.name,
            values,
            format_code,
            // 折线的颜色是线色,其余取填充色。
            color: if kind == ChartKind::Line {
                rs.line
            } else {
                rs.fill
            },
            point_colors: {
                // 同一 idx 重复时取文档顺序第一个(稳定排序 + 去重保留首个),结果升序唯一。
                let mut pc = rs.point_colors;
                pc.sort_by_key(|(i, _)| *i);
                pc.dedup_by_key(|(i, _)| *i);
                pc
            },
            labels: rs
                .labels
                .filter(|l| l.show_val || l.show_cat_name || l.show_percent),
        });
    }

    chart.title = match title {
        Some(Some(t)) => Some(t),
        // 有 c:title 但无文字:PowerPoint 对单系列图显示系列名作自动标题。
        Some(None) if !auto_deleted && chart.series.len() == 1 => chart.series[0].name.clone(),
        _ => None,
    };
}

fn val_true(e: &BytesStart) -> bool {
    attr_of(e, b"val").is_none_or(super::ooxml_bool)
}

/// `c:title`:返回 `c:tx` 的纯文本;无 `c:tx`(或文字为空)为 `None`。已消费起始标签。
fn parse_title<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<String> {
    let mut text = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"tx" {
                    text = parse_tx(reader);
                } else {
                    skip_element(reader, &name);
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    text
}

/// `c:tx`(标题 / 系列名):`c:rich` 富文本(段落以空格连接)、`c:strRef > c:strCache`
/// 或直接 `c:v`。空文字为 `None`。已消费起始标签。
fn parse_tx<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<String> {
    let mut text = String::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"rich" => text = parse_rich(reader),
                    b"strRef" => {
                        let mut st = State {
                            warnings: Vec::new(),
                            budget: MAX_POINTS,
                        };
                        let cache = parse_source_el(reader, &mut st);
                        text = cache
                            .points
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" ");
                    }
                    b"v" => text = read_text(reader),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    let t = text.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// `c:rich`(`a:bodyPr` + `a:p*`):各段 `a:t` 文字,段间以空格连接。已消费起始标签。
fn parse_rich<R: std::io::BufRead>(reader: &mut Reader<R>) -> String {
    let mut paras: Vec<String> = Vec::new();
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match local_name(e.name().as_ref()) {
                b"p" => {
                    paras.push(String::new());
                    depth += 1;
                }
                b"t" => {
                    let t = read_text(reader);
                    match paras.last_mut() {
                        Some(p) => p.push_str(&t),
                        None => paras.push(t),
                    }
                }
                _ => depth += 1,
            },
            Ok(Event::End(_)) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    paras
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 一个图类型元素的布局细节(`c:barDir` / `c:grouping` / 是否 3D 变体)。
#[derive(Debug, Default)]
struct PlotInfo {
    bar_dir: Option<String>,
    grouping: Option<String>,
    three_d: bool,
    /// `c:ofPieChart`(复合饼)。
    of_pie: bool,
    /// 类型级 `c:dLbls`(系列未自设时继承)。
    labels: Option<DataLabels>,
}

/// 图类型元素本地名是否为 3D 变体(如 `bar3DChart`)。
fn is_3d(name: &str) -> bool {
    name.strip_suffix("Chart")
        .is_some_and(|b| b.ends_with("3D"))
}

/// `c:plotArea`:每个 `c:*Chart` 图类型元素记一个种类(+ 布局细节),收集其 `c:ser`。
/// 已消费起始标签。
fn parse_plot_area<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    kinds: &mut Vec<(ChartKind, PlotInfo)>,
    raw: &mut Vec<(ChartKind, RawSeries)>,
    st: &mut State,
) {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                let el = String::from_utf8_lossy(&name).into_owned();
                match ChartKind::from_element(&el) {
                    Some(kind) => {
                        let mut info = parse_plot(reader, &kind, raw, st);
                        info.three_d = is_3d(&el);
                        info.of_pie = el == "ofPieChart";
                        kinds.push((kind, info));
                    }
                    None => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(local_name(e.name().as_ref())).into_owned();
                if let Some(kind) = ChartKind::from_element(&name) {
                    let info = PlotInfo {
                        three_d: is_3d(&name),
                        of_pie: name == "ofPieChart",
                        ..PlotInfo::default()
                    };
                    kinds.push((kind, info));
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
}

/// 一个图类型元素(如 `c:barChart`):收集其 `c:ser`,返回 `c:barDir` / `c:grouping`。
/// 已消费起始标签。
fn parse_plot<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    kind: &ChartKind,
    raw: &mut Vec<(ChartKind, RawSeries)>,
    st: &mut State,
) -> PlotInfo {
    let mut info = PlotInfo::default();
    let first = raw.len();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"dLbls" {
                    info.labels = Some(parse_dlbls(reader));
                } else if name.as_slice() == b"ser" && raw.len() < MAX_SERIES {
                    raw.push((kind.clone(), parse_ser(reader, st)));
                } else {
                    if name.as_slice() == b"ser" {
                        st.warn(format!(
                            "chart has more than {MAX_SERIES} series; rest dropped"
                        ));
                    }
                    skip_element(reader, &name);
                }
            }
            Ok(Event::Empty(e)) => match local_name(e.name().as_ref()) {
                b"barDir" => info.bar_dir = attr_of(&e, b"val"),
                b"grouping" => info.grouping = attr_of(&e, b"val"),
                _ => {}
            },
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    // `c:dLbls` 排在 `c:ser` 之后:读完整个图类型元素再把类型级设置填给没自设的系列。
    if let Some(group) = info.labels {
        for (_, rs) in &mut raw[first..] {
            rs.labels.get_or_insert(group);
        }
    }
    info
}

/// `c:ser`:系列名 `c:tx`、类别 `c:cat` / `c:xVal`、值 `c:val` / `c:yVal`。已消费起始标签。
fn parse_ser<R: std::io::BufRead>(reader: &mut Reader<R>, st: &mut State) -> RawSeries {
    let mut s = RawSeries::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"tx" => s.name = parse_tx(reader),
                    b"cat" | b"xVal" => s.cat = Some(parse_data_source(reader, st)),
                    b"val" | b"yVal" => s.val = Some(parse_data_source(reader, st)),
                    b"spPr" => (s.fill, s.line) = parse_sppr_colors(reader),
                    b"dPt" => {
                        if let (Some(idx), Some(color)) = parse_dpt(reader) {
                            s.point_colors.push((idx, color));
                        }
                    }
                    b"dLbls" => s.labels = Some(parse_dlbls(reader)),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => match local_name(e.name().as_ref()) {
                b"cat" | b"xVal" => s.cat = Some(Cache::default()),
                b"val" | b"yVal" => s.val = Some(Cache::default()),
                _ => {}
            },
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    s
}

/// `c:spPr`:直接子 `a:solidFill` 的颜色与 `a:ln > a:solidFill` 的颜色;渐变 / 图案 / `noFill`
/// 一律跳过(取不到颜色,渲染回落 accent)。已消费起始标签。
fn parse_sppr_colors<R: std::io::BufRead>(
    reader: &mut Reader<R>,
) -> (Option<ColorSpec>, Option<ColorSpec>) {
    let (mut fill, mut line) = (None, None);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"solidFill" => fill = parse_solid_fill(reader),
                    b"ln" => line = parse_ln(reader, &e).and_then(|s| s.color),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (fill, line)
}

/// `c:dPt`:`c:idx@val` + `c:spPr > a:solidFill`。已消费起始标签。
fn parse_dpt<R: std::io::BufRead>(reader: &mut Reader<R>) -> (Option<usize>, Option<ColorSpec>) {
    let (mut idx, mut color) = (None, None);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"spPr" {
                    color = parse_sppr_colors(reader).0;
                } else {
                    skip_element(reader, &name);
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"idx" {
                    idx = attr_of(&e, b"val").and_then(|v| v.trim().parse().ok());
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (idx.filter(|i| *i < MAX_POINTS), color)
}

/// `c:dLbls`:`c:showVal` / `c:showCatName` / `c:showPercent`(未写 = false);`c:delete` = 全关;
/// `c:numFmt` / 逐点 `c:dLbl` 等其余子元素跳过。已消费起始标签。
fn parse_dlbls<R: std::io::BufRead>(reader: &mut Reader<R>) -> DataLabels {
    let mut l = DataLabels::default();
    let mut deleted = false;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                skip_element(reader, &name);
            }
            Ok(Event::Empty(e)) => match local_name(e.name().as_ref()) {
                b"showVal" => l.show_val = val_true(&e),
                b"showCatName" => l.show_cat_name = val_true(&e),
                b"showPercent" => l.show_percent = val_true(&e),
                b"delete" => deleted = val_true(&e),
                _ => {}
            },
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    if deleted {
        DataLabels::default()
    } else {
        l
    }
}

/// 数据源元素(`c:cat` / `c:val` / …)内的 `strRef` / `numRef` / `strLit` / `numLit` /
/// `multiLvlStrRef`。已消费起始标签。
fn parse_data_source<R: std::io::BufRead>(reader: &mut Reader<R>, st: &mut State) -> Cache {
    let mut cache = Cache::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"strRef" | b"numRef" | b"multiLvlStrRef" => {
                        cache = parse_source_el(reader, st);
                    }
                    b"strLit" | b"numLit" => cache = parse_points(reader, st),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                if matches!(local_name(e.name().as_ref()), b"strLit" | b"numLit") {
                    cache.found = true;
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    cache
}

/// `*Ref` 元素:跳过公式 `c:f`,取 `strCache` / `numCache`(`multiLvlStrCache` 取首个 `c:lvl`)。
/// 已消费起始标签。
fn parse_source_el<R: std::io::BufRead>(reader: &mut Reader<R>, st: &mut State) -> Cache {
    let mut cache = Cache::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"strCache" | b"numCache" => cache = parse_points(reader, st),
                    b"multiLvlStrCache" => cache = parse_multi_lvl(reader, st),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                if matches!(
                    local_name(e.name().as_ref()),
                    b"strCache" | b"numCache" | b"multiLvlStrCache"
                ) {
                    cache.found = true;
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    cache
}

/// `c:multiLvlStrCache`:`ptCount` + 若干 `c:lvl`(首层 = 最内层类别,取它)。已消费起始标签。
fn parse_multi_lvl<R: std::io::BufRead>(reader: &mut Reader<R>, st: &mut State) -> Cache {
    let mut count: Option<usize> = None;
    let mut first: Option<Cache> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"lvl" && first.is_none() {
                    first = Some(parse_points(reader, st));
                } else {
                    if name.as_slice() == b"ptCount" {
                        count = pt_count(&e);
                    }
                    skip_element(reader, &name);
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"ptCount" {
                    count = pt_count(&e);
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    let mut cache = first.unwrap_or_default();
    cache.found = true;
    if let Some(n) = count {
        pad(&mut cache.points, n, st);
    }
    cache
}

fn pt_count(e: &BytesStart) -> Option<usize> {
    attr_of(e, b"val").and_then(|v| v.trim().parse().ok())
}

/// 缓存 / 字面量体:`c:formatCode` + `c:ptCount` + `c:pt@idx > c:v`。按 `ptCount` 补齐稀疏点。
/// 已消费起始标签。
fn parse_points<R: std::io::BufRead>(reader: &mut Reader<R>, st: &mut State) -> Cache {
    let mut cache = Cache {
        found: true,
        ..Cache::default()
    };
    let mut count: Option<usize> = None;
    let mut pts: Vec<(usize, String)> = Vec::new();
    let mut next_idx = 0usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"formatCode" => {
                        let t = read_text(reader);
                        let t = t.trim();
                        if !t.is_empty() {
                            cache.format_code = Some(t.to_string());
                        }
                    }
                    b"ptCount" => {
                        count = pt_count(&e);
                        skip_element(reader, &name);
                    }
                    b"pt" => {
                        let idx = attr_of(&e, b"idx")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(next_idx);
                        next_idx = idx.saturating_add(1);
                        let v = read_pt_value(reader);
                        if idx < MAX_POINTS {
                            pts.push((idx, v));
                        } else {
                            st.warn(format!("chart point index {idx} exceeds limit; dropped"));
                        }
                    }
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"ptCount" {
                    count = pt_count(&e);
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    let len = pts
        .iter()
        .map(|(i, _)| i + 1)
        .max()
        .unwrap_or(0)
        .max(count.unwrap_or(0).min(MAX_POINTS));
    pad(&mut cache.points, len, st);
    let n = cache.points.len();
    for (idx, v) in pts {
        if idx < n {
            cache.points[idx] = Some(v);
        }
    }
    cache
}

/// 把点列补齐到 `len`(受整图点数预算约束,超出截断并告警)。
fn pad(points: &mut Vec<Option<String>>, len: usize, st: &mut State) {
    if len <= points.len() {
        return;
    }
    let extra = len - points.len();
    let take = extra.min(st.budget);
    if take < extra {
        st.warn("chart data exceeds point budget; truncated".to_string());
    }
    st.budget -= take;
    points.resize(points.len() + take, None);
}

/// `c:pt` 内的 `c:v` 文本。已消费 `<c:pt>` 起始标签。
fn read_pt_value<R: std::io::BufRead>(reader: &mut Reader<R>) -> String {
    let mut v = String::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"v" {
                    v = read_text(reader);
                } else {
                    skip_element(reader, &name);
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space(plot: &str, title: &str) -> String {
        format!(
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
               xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
              <c:chart>{title}<c:autoTitleDeleted val="0"/>
                <c:plotArea><c:layout/>{plot}
                  <c:valAx><c:title><c:tx><c:rich><a:p><a:r><a:t>AXIS</a:t></a:r></a:p></c:rich></c:tx></c:title></c:valAx>
                </c:plotArea>
              </c:chart>
            </c:chartSpace>"#
        )
    }

    fn str_cache(vals: &[&str]) -> String {
        let pts: String = vals
            .iter()
            .enumerate()
            .map(|(i, v)| format!(r#"<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>"#))
            .collect();
        format!(
            r#"<c:strRef><c:f>Sheet1!$A$2</c:f><c:strCache><c:ptCount val="{}"/>{pts}</c:strCache></c:strRef>"#,
            vals.len()
        )
    }

    #[test]
    fn bar_chart_with_rich_title_and_two_series() {
        let ser = |name: &str, a: &str, b: &str| {
            format!(
                r#"<c:ser><c:idx val="0"/><c:tx>{}</c:tx><c:cat>{}</c:cat>
                <c:val><c:numRef><c:f>x</c:f><c:numCache><c:formatCode>General</c:formatCode>
                <c:ptCount val="2"/><c:pt idx="0"><c:v>{a}</c:v></c:pt><c:pt idx="1"><c:v>{b}</c:v></c:pt>
                </c:numCache></c:numRef></c:val></c:ser>"#,
                str_cache(&[name]),
                str_cache(&["Q1", "Q2"])
            )
        };
        let xml = space(
            &format!(
                r#"<c:barChart><c:barDir val="col"/>{}{}<c:axId val="1"/></c:barChart>"#,
                ser("North", "1", "2.5"),
                ser("South", "3", "4")
            ),
            r#"<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Sales </a:t></a:r><a:r><a:t>2024</a:t></a:r></a:p></c:rich></c:tx></c:title>"#,
        );
        let c = parse(&xml);
        assert_eq!(c.kind, ChartKind::Bar);
        assert_eq!(c.title.as_deref(), Some("Sales 2024"));
        assert_eq!(c.categories, vec!["Q1", "Q2"]);
        assert_eq!(c.series.len(), 2);
        assert_eq!(c.series[0].name.as_deref(), Some("North"));
        assert_eq!(c.series[0].values, vec![Some(1.0), Some(2.5)]);
        assert_eq!(c.series[1].values, vec![Some(3.0), Some(4.0)]);
        assert_eq!(c.series[0].format_code.as_deref(), Some("General"));
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    }

    #[test]
    fn sparse_points_pad_to_pt_count() {
        let xml = space(
            r#"<c:lineChart><c:ser><c:val><c:numLit><c:ptCount val="4"/>
               <c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="3"><c:v>x</c:v></c:pt></c:numLit></c:val></c:ser></c:lineChart>"#,
            "",
        );
        let c = parse(&xml);
        assert_eq!(c.kind, ChartKind::Line);
        assert_eq!(c.series[0].values, vec![Some(1.0), None, None, None]);
        assert_eq!(c.title, None);
    }

    #[test]
    fn missing_cache_warns_and_keeps_empty_series() {
        let xml = space(
            r#"<c:areaChart><c:ser><c:tx><c:v>Lit</c:v></c:tx>
               <c:cat><c:strRef><c:f>A</c:f></c:strRef></c:cat>
               <c:val><c:numRef><c:f>B</c:f></c:numRef></c:val></c:ser></c:areaChart>"#,
            "",
        );
        let c = parse(&xml);
        assert_eq!(c.series[0].name.as_deref(), Some("Lit"));
        assert!(c.series[0].values.is_empty());
        assert_eq!(c.warnings.len(), 2, "{:?}", c.warnings);
        assert!(c.warnings[1].contains("val cache missing"));
    }

    #[test]
    fn auto_title_uses_single_series_name_and_unknown_kind() {
        let xml = space(
            &format!(
                r#"<c:fooChart><c:ser><c:tx>{}</c:tx><c:val><c:numLit><c:pt idx="0"><c:v>7</c:v></c:pt></c:numLit></c:val></c:ser></c:fooChart>"#,
                str_cache(&["Only"])
            ),
            "<c:title><c:overlay val=\"0\"/></c:title>",
        );
        let c = parse(&xml);
        assert_eq!(c.kind, ChartKind::Other("fooChart".into()));
        assert_eq!(c.title.as_deref(), Some("Only"));
        assert_eq!(c.series[0].values, vec![Some(7.0)]);
    }

    /// 主图类型的布局细节:`c:barDir` / `c:grouping`、3D 变体、组合图(多个图类型元素)。
    #[test]
    fn plot_layout_details_bar_dir_grouping_3d_and_combo() {
        let ser = r#"<c:ser><c:val><c:numLit><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser>"#;
        let c = parse(&space(
            &format!(
                r#"<c:barChart><c:barDir val="bar"/><c:grouping val="stacked"/>{ser}</c:barChart>"#
            ),
            "",
        ));
        assert_eq!(c.bar_dir.as_deref(), Some("bar"));
        assert_eq!(c.grouping.as_deref(), Some("stacked"));
        assert!(!c.three_d && !c.combo);

        let c = parse(&space(
            &format!(r#"<c:bar3DChart><c:barDir val="col"/>{ser}</c:bar3DChart>"#),
            "",
        ));
        assert_eq!(c.kind, ChartKind::Bar);
        assert_eq!(c.bar_dir.as_deref(), Some("col"));
        assert_eq!(c.grouping, None);
        assert!(c.three_d);

        // 组合图:主类型取首个,细节也取首个;第二个图类型的 grouping 不覆盖。
        let c = parse(&space(
            &format!(
                r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{ser}</c:barChart>
                   <c:lineChart><c:grouping val="standard"/>{ser}</c:lineChart>"#
            ),
            "",
        ));
        assert!(c.combo);
        assert_eq!(c.grouping.as_deref(), Some("clustered"));
        assert_eq!(c.series.len(), 2);

        let c = parse(&space(&format!(r#"<c:pieChart>{ser}</c:pieChart>"#), ""));
        assert_eq!(
            (c.bar_dir, c.grouping, c.three_d, c.combo),
            (None, None, false, false)
        );
    }

    /// `c:ser > c:spPr`:柱取 solidFill、折线取 `a:ln` 的 solidFill(含 schemeClr 与变换原样保留);
    /// 渐变 / 图案 / noFill 跳过;`c:dPt` 逐点色。
    #[test]
    fn series_and_point_colors_are_parsed() {
        use ppt_core::color::ColorSpec;
        let val = r#"<c:val><c:numLit><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val>"#;
        let c = parse(&space(
            &format!(
                r#"<c:barChart><c:barDir val="col"/>
                <c:ser><c:spPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:ln><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></a:ln></c:spPr>{val}</c:ser>
                <c:ser><c:spPr><a:solidFill><a:schemeClr val="accent2"><a:lumMod val="75000"/></a:schemeClr></a:solidFill></c:spPr>{val}</c:ser>
                <c:ser><c:spPr><a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="0000FF"/></a:gs></a:gsLst></a:gradFill></c:spPr>{val}</c:ser>
                <c:ser><c:spPr><a:pattFill prst="pct5"><a:fgClr><a:srgbClr val="0000FF"/></a:fgClr></a:pattFill></c:spPr>{val}</c:ser>
                <c:ser>{val}</c:ser></c:barChart>"#
            ),
            "",
        ));
        let colors: Vec<_> = c.series.iter().map(|s| s.color.clone()).collect();
        assert_eq!(
            colors[0],
            Some(ColorSpec::srgb([0xFF, 0, 0])),
            "柱取 solidFill 而非 ln"
        );
        assert!(
            matches!(&colors[1], Some(ColorSpec::Scheme { name, transforms }) if name == "accent2" && transforms.len() == 1),
            "schemeClr + 变换原样保留: {:?}",
            colors[1]
        );
        assert_eq!(
            &colors[2..],
            &[None, None, None],
            "渐变 / 图案 / 无 spPr 跳过"
        );

        let c = parse(&space(
            &format!(
                r#"<c:lineChart><c:ser><c:spPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:ln w="28575"><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></a:ln></c:spPr>{val}</c:ser></c:lineChart>"#
            ),
            "",
        ));
        assert_eq!(
            c.series[0].color,
            Some(ColorSpec::srgb([0, 0xFF, 0])),
            "折线取 ln 颜色"
        );

        let c = parse(&space(
            &format!(
                r#"<c:pieChart><c:varyColors val="1"/><c:ser>{val}
                <c:dPt><c:idx val="0"/><c:bubble3D val="0"/><c:spPr><a:solidFill><a:srgbClr val="112233"/></a:solidFill></c:spPr></c:dPt>
                <c:dPt><c:idx val="2"/><c:spPr><a:gradFill/></c:spPr></c:dPt>
                <c:dPt><c:idx val="3"/><c:spPr><a:solidFill><a:srgbClr val="445566"/></a:solidFill></c:spPr></c:dPt>
                </c:ser></c:pieChart>"#
            ),
            "",
        ));
        assert_eq!(
            c.series[0].point_colors,
            vec![
                (0, ColorSpec::srgb([0x11, 0x22, 0x33])),
                (3, ColorSpec::srgb([0x44, 0x55, 0x66]))
            ]
        );
    }

    /// `c:dPt@idx` 重复:取文档顺序第一个,且结果按 idx 升序、唯一(渲染与 Python 绑定共用这条规则)。
    #[test]
    fn duplicate_point_color_idx_keeps_first_and_sorts() {
        use ppt_core::color::ColorSpec;
        let dpt = |idx: u32, rgb: &str| {
            format!(
                r#"<c:dPt><c:idx val="{idx}"/><c:spPr><a:solidFill><a:srgbClr val="{rgb}"/></a:solidFill></c:spPr></c:dPt>"#
            )
        };
        let val = r#"<c:val><c:numLit><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val>"#;
        let c = parse(&space(
            &format!(
                "<c:pieChart><c:ser>{val}{}{}{}{}</c:ser></c:pieChart>",
                dpt(3, "030303"),
                dpt(1, "111111"),
                dpt(1, "222222"),
                dpt(3, "333333")
            ),
            "",
        ));
        assert_eq!(
            c.series[0].point_colors,
            vec![
                (1, ColorSpec::srgb([0x11, 0x11, 0x11])),
                (3, ColorSpec::srgb([0x03, 0x03, 0x03]))
            ]
        );
    }

    /// `c:dLbls`:系列级整体覆盖图表类型级;`c:delete` 关闭;未写的 show* 视为 false;
    /// `c:numFmt` 非 General 不影响解析。
    #[test]
    fn data_labels_series_overrides_group() {
        use ppt_core::model::DataLabels;
        let val = r#"<c:val><c:numLit><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val>"#;
        let c = parse(&space(
            &format!(
                r#"<c:barChart><c:barDir val="col"/>
                <c:ser>{val}</c:ser>
                <c:ser><c:dLbls><c:numFmt formatCode="0.0%" sourceLinked="0"/><c:showVal val="0"/><c:showCatName val="1"/></c:dLbls>{val}</c:ser>
                <c:ser><c:dLbls><c:delete val="1"/></c:dLbls>{val}</c:ser>
                <c:dLbls><c:numFmt formatCode="0.00" sourceLinked="0"/><c:showLegendKey val="0"/><c:showVal val="1"/><c:showCatName val="0"/><c:showPercent val="0"/></c:dLbls>
                </c:barChart>
                <c:lineChart><c:ser>{val}</c:ser></c:lineChart>"#
            ),
            "",
        ));
        let l: Vec<_> = c.series.iter().map(|s| s.labels).collect();
        assert_eq!(
            l[0],
            Some(DataLabels {
                show_val: true,
                ..DataLabels::default()
            }),
            "继承图表类型级"
        );
        assert_eq!(
            l[1],
            Some(DataLabels {
                show_cat_name: true,
                ..DataLabels::default()
            }),
            "系列级整体覆盖(showVal 未设 = false)"
        );
        assert_eq!(l[2], None, "delete = 无标签");
        assert_eq!(l[3], None, "另一图类型不继承 barChart 的 dLbls");

        let c = parse(&space(
            &format!(
                r#"<c:pieChart><c:ser>{val}<c:dLbls><c:showPercent val="1"/></c:dLbls></c:ser></c:pieChart>"#
            ),
            "",
        ));
        assert_eq!(
            c.series[0].labels,
            Some(DataLabels {
                show_percent: true,
                ..DataLabels::default()
            })
        );
    }

    /// `c:ofPieChart` 仍并入 `Pie`,但带 `of_pie` 标记(渲染据此降级占位框)。
    #[test]
    fn of_pie_is_marked() {
        let ser = r#"<c:ser><c:val><c:numLit><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser>"#;
        let c = parse(&space(
            &format!("<c:ofPieChart><c:ofPieType val=\"pie\"/>{ser}</c:ofPieChart>"),
            "",
        ));
        assert_eq!(c.kind, ChartKind::Pie);
        assert!(c.of_pie);
        let c = parse(&space(&format!("<c:pieChart>{ser}</c:pieChart>"), ""));
        assert!(!c.of_pie);
    }

    #[test]
    fn huge_pt_count_is_capped() {
        let xml = space(
            r#"<c:pie3DChart><c:ser><c:val><c:numLit><c:ptCount val="99999999999"/>
               <c:pt idx="4294967295"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser></c:pie3DChart>"#,
            "",
        );
        let c = parse(&xml);
        assert_eq!(c.kind, ChartKind::Pie);
        assert_eq!(c.series[0].values.len(), MAX_POINTS);
        assert!(!c.warnings.is_empty());
    }
}
