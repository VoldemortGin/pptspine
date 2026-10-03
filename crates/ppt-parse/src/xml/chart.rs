//! 解析图表部件 `ppt/charts/chartN.xml`(`c:chartSpace`)-> [`Chart`]。
//!
//! 只读部件内的**缓存**:标题(`c:title > c:tx` 的 `c:rich` 富文本或 `c:strRef > c:strCache`)、
//! `c:plotArea` 下各图类型的系列(`c:ser`):系列名 `c:tx`、类别 `c:cat`(散点 / 气泡为 `c:xVal`)、
//! 值 `c:val`(散点 / 气泡为 `c:yVal`)。数据源支持 `strRef` / `numRef`(取其 cache)、
//! `strLit` / `numLit`、`multiLvlStrRef`(取首层)。`pt@idx` 可能稀疏,按 `ptCount` 补空。
//! **不**解析外部工作簿;缺缓存的系列记为空并写入 [`Chart::warnings`]。
//!
//! 容错:未知元素跳过、畸形数字 → 缺点、绝不 panic;点数 / 系列数设上限防放大攻击。

use ppt_core::model::{Chart, ChartKind, ChartSeries};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

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
    let mut kinds: Vec<ChartKind> = Vec::new();
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

    if let Some(k) = kinds.into_iter().next() {
        chart.kind = k;
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

/// `c:plotArea`:每个 `c:*Chart` 图类型元素记一个种类,收集其 `c:ser`。已消费起始标签。
fn parse_plot_area<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    kinds: &mut Vec<ChartKind>,
    raw: &mut Vec<(ChartKind, RawSeries)>,
    st: &mut State,
) {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match ChartKind::from_element(&String::from_utf8_lossy(&name)) {
                    Some(kind) => {
                        kinds.push(kind.clone());
                        parse_plot(reader, &kind, raw, st);
                    }
                    None => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(local_name(e.name().as_ref())).into_owned();
                if let Some(kind) = ChartKind::from_element(&name) {
                    kinds.push(kind);
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
}

/// 一个图类型元素(如 `c:barChart`):收集其 `c:ser`。已消费起始标签。
fn parse_plot<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    kind: &ChartKind,
    raw: &mut Vec<(ChartKind, RawSeries)>,
    st: &mut State,
) {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"ser" && raw.len() < MAX_SERIES {
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
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
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
