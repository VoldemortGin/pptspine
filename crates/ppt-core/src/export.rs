//! 结构化导出:把已解析的模型渲染成纯文本 / Markdown。
//!
//! 基于现有领域模型(无 IO / XML),逐 slide 拼接文字、表格,并可选纳入演讲者备注。
//! 设计目标:确定性、容错(空内容不产生噪声)、对合并单元格保真(Markdown 走 HTML `<table>`)。
//!
//! 分层:[`reading_order`] 展平组合 + 视觉阅读顺序;`view` 导出选项 / 有序形状视图 / 纯文本;
//! `markdown` 语义 Markdown(标题占位符、列表标记、图片、超链接)。本文件保留表格与共用辅助。

mod markdown;
pub mod reading_order;
mod view;

use crate::model::{
    Cell, Chart, ChartKind, Paragraph, Presentation, RunKind, Slide, Table, TextFrame, TextRun,
};
use crate::resolved::{ResolvedCell, ResolvedParagraph, ResolvedRun, ResolvedTable};

pub use markdown::presentation_markdown_with;
pub use view::{presentation_text_with, slide_text_with, ExportOptions, TextOrder};

/// 一张幻灯片的正文文字(所有文本框 / 自选图形文字 / 表格,按视觉阅读顺序;**不含备注**)。
/// 无继承链信息(占位符继承几何缺失的形状排在最后);需要更准的顺序用 [`slide_text_with`]。
pub fn slide_text(slide: &Slide) -> String {
    slide_text_with(slide, None, (0, 0), TextOrder::default())
}

/// 整份演示文稿的纯文本(缺省选项:视觉顺序、跳过隐藏页;见 [`presentation_text_with`])。
pub fn presentation_text(pres: &Presentation) -> String {
    presentation_text_with(pres, None, &ExportOptions::default())
}

/// 整份演示文稿的 Markdown(缺省选项;见 [`presentation_markdown_with`])。
pub fn presentation_markdown(pres: &Presentation) -> String {
    presentation_markdown_with(pres, None, &ExportOptions::default())
}

// ---- 纯文本辅助 ----------------------------------------------------------

/// 文本体纯文本。`resolved` 为与 `tf.paragraphs` 一一对应的终态段落(有则字段 run 取其求值结果)。
fn frame_text(tf: &TextFrame, resolved: Option<&[ResolvedParagraph]>) -> String {
    tf.paragraphs
        .iter()
        .enumerate()
        .map(|(i, p)| paragraph_text_with(p, resolved.map(|r| &r[i])))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 段落纯文本;字段 run(`a:fld`)有终态 run 时取其文字——`slidenum` 的页码求值只在
/// `resolve.rs` 一处(文本导出与 PDF 共用),`datetime*` 在那里保持缓存文本。无终态 IR 时退回缓存文本。
fn paragraph_text_with(p: &Paragraph, resolved: Option<&ResolvedParagraph>) -> String {
    p.runs
        .iter()
        .enumerate()
        .map(|(i, r)| run_text(r, resolved.and_then(|rp| rp.runs.get(i))))
        .collect()
}

/// 一个 run 的导出文字:字段 run 且有终态 run → 终态文字,否则原文。
fn run_text<'a>(run: &'a TextRun, resolved: Option<&'a ResolvedRun>) -> &'a str {
    match (&run.kind, resolved) {
        (RunKind::Field { .. }, Some(rr)) => &rr.text,
        _ => &run.text,
    }
}

/// 单元格纯文本;`resolved` 为同位置的终态单元格(段落按下标配对,字段 run 取其求值结果)。
fn cell_text(c: &Cell, resolved: Option<&ResolvedCell>) -> String {
    c.paragraphs
        .iter()
        .enumerate()
        .map(|(i, p)| paragraph_text_with(p, resolved.and_then(|rc| rc.paragraphs.get(i))))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 终态表格里 `(行, 列)` 处的单元格(终态逐行逐格与原表一一对应;越界 → `None`)。
fn resolved_cell(rt: Option<&ResolvedTable>, ri: usize, ci: usize) -> Option<&ResolvedCell> {
    rt?.rows.get(ri)?.cells.get(ci)
}

fn table_text(t: &Table, rt: Option<&ResolvedTable>) -> String {
    let mut lines: Vec<String> = Vec::new();
    for (ri, row) in t.rows.iter().enumerate() {
        let cells: Vec<String> = row
            .cells
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.merged) // 被合并掉的延续格不重复输出
            .map(|(ci, c)| cell_text(c, resolved_cell(rt, ri, ci)))
            .collect();
        lines.push(cells.join(" | "));
    }
    lines.join("\n")
}

fn notes_text(slide: &Slide) -> Option<String> {
    match &slide.notes {
        Some(n) if !n.trim().is_empty() => Some(n.clone()),
        _ => None,
    }
}

// ---- 图表辅助 -----------------------------------------------------------

/// 图表标题行文字:有标题用标题,否则 `Chart (<kind>)`。
fn chart_label(c: &Chart) -> String {
    match &c.title {
        Some(t) => format!("Chart: {}", one_line_text(t)),
        None => format!("Chart ({})", c.kind.name()),
    }
}

fn one_line_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 图表还原成表格:表头(首列类别 / X,其余各系列名)+ 数据行(首列类别,其余为
/// 按 `format_code` 格式化的值;缺点为空串)。无系列时为 `None`。
fn chart_table(c: &Chart) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    if c.series.is_empty() {
        return None;
    }
    let first = if matches!(c.kind, ChartKind::Scatter | ChartKind::Bubble) {
        "X"
    } else {
        "Category"
    };
    let mut header = vec![first.to_string()];
    header.extend(c.series.iter().enumerate().map(|(i, s)| {
        s.name
            .as_deref()
            .map(one_line_text)
            .unwrap_or_else(|| format!("Series {}", i + 1))
    }));
    let n = c
        .series
        .iter()
        .map(|s| s.values.len())
        .max()
        .unwrap_or(0)
        .max(c.categories.len());
    let rows = (0..n)
        .map(|i| {
            let cat = match c.categories.get(i) {
                Some(s) => one_line_text(s),
                None if c.categories.is_empty() => (i + 1).to_string(),
                None => String::new(),
            };
            let mut row = vec![cat];
            row.extend(c.series.iter().map(|s| {
                s.values
                    .get(i)
                    .copied()
                    .flatten()
                    .map(|v| format_number(v, s.format_code.as_deref()))
                    .unwrap_or_default()
            }));
            row
        })
        .collect();
    Some((header, rows))
}

/// 按 Excel 数字格式粗略格式化:`General` / 无格式 / 非数字格式 → 最短往返表示(整数不带
/// `.0`);含 `0`/`#` 占位的格式按小数位数、千分位(`#,##0`)与百分号(`0.0%`)格式化。
fn format_number(v: f64, code: Option<&str>) -> String {
    let general = || {
        if v.fract() == 0.0 && v.abs() < 1e15 {
            format!("{v:.0}")
        } else {
            v.to_string()
        }
    };
    let Some(code) = code else {
        return general();
    };
    // 只看正数段;去掉 `"…"` 字面量与 `[…]` 修饰(颜色 / 区域)。
    let section = code.split(';').next().unwrap_or("");
    let mut fmt = String::new();
    let (mut in_quote, mut in_bracket) = (false, false);
    for ch in section.chars() {
        match ch {
            '"' => in_quote = !in_quote,
            '[' if !in_quote => in_bracket = true,
            ']' if !in_quote => in_bracket = false,
            _ if !in_quote && !in_bracket => fmt.push(ch),
            _ => {}
        }
    }
    if section.eq_ignore_ascii_case("general") || !fmt.contains(['0', '#']) {
        return general();
    }
    let percent = fmt.contains('%');
    let decimals = fmt
        .split_once('.')
        .map(|(_, frac)| frac.chars().take_while(|c| matches!(c, '0' | '#')).count())
        .unwrap_or(0);
    let int_part = fmt.split('.').next().unwrap_or("");
    let grouping = int_part.contains(",#") || int_part.contains(",0");
    let x = if percent { v * 100.0 } else { v };
    let mut s = format!("{x:.decimals$}");
    if grouping {
        s = group_thousands(&s);
    }
    if percent {
        s.push('%');
    }
    s
}

/// 给定点数字串的整数部分加千分位逗号。
fn group_thousands(s: &str) -> String {
    let (sign, rest) = s.strip_prefix('-').map_or(("", s), |r| ("-", r));
    let (int, frac) = rest
        .split_once('.')
        .map_or((rest, None), |(i, f)| (i, Some(f)));
    let mut grouped = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

// ---- Markdown 辅助 -------------------------------------------------------

fn table_markdown(t: &Table, rt: Option<&ResolvedTable>) -> String {
    if t.rows.is_empty() {
        return String::new();
    }
    let has_merge = t
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .any(|c| c.col_span > 1 || c.row_span > 1 || c.merged);
    if has_merge {
        table_html(t, rt)
    } else {
        table_gfm(t, rt)
    }
}

fn table_gfm(t: &Table, rt: Option<&ResolvedTable>) -> String {
    let cols = t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    if cols == 0 {
        return String::new();
    }
    let mut lines: Vec<String> = Vec::new();
    for (i, row) in t.rows.iter().enumerate() {
        let mut cells: Vec<String> = row
            .cells
            .iter()
            .enumerate()
            .map(|(ci, c)| escape_pipe(&cell_text(c, resolved_cell(rt, i, ci))))
            .collect();
        while cells.len() < cols {
            cells.push(String::new());
        }
        lines.push(format!("| {} |", cells.join(" | ")));
        if i == 0 {
            let sep = vec!["---"; cols].join(" | ");
            lines.push(format!("| {sep} |"));
        }
    }
    lines.join("\n")
}

fn table_html(t: &Table, rt: Option<&ResolvedTable>) -> String {
    let mut s = String::from("<table>");
    for (ri, row) in t.rows.iter().enumerate() {
        s.push_str("\n  <tr>");
        for (ci, c) in row.cells.iter().enumerate() {
            if c.merged {
                continue; // 被合并掉的延续格不输出,跨度由主格的 colspan/rowspan 表达
            }
            let mut attrs = String::new();
            if c.col_span > 1 {
                attrs.push_str(&format!(" colspan=\"{}\"", c.col_span));
            }
            if c.row_span > 1 {
                attrs.push_str(&format!(" rowspan=\"{}\"", c.row_span));
            }
            s.push_str(&format!(
                "\n    <td{attrs}>{}</td>",
                escape_html(&cell_text(c, resolved_cell(rt, ri, ci)))
            ));
        }
        s.push_str("\n  </tr>");
    }
    s.push_str("\n</table>");
    s
}

/// 转义 GFM 表格单元格里的管道符与换行,避免破坏行结构。
fn escape_pipe(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', "<br>")
}

/// 最小 HTML 转义(`&`/`<`/`>`)。
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_follow_format_code_roughly() {
        assert_eq!(format_number(3.0, None), "3");
        assert_eq!(format_number(2.5, Some("General")), "2.5");
        assert_eq!(format_number(-12.0, Some("General")), "-12");
        assert_eq!(format_number(0.125, Some("0.0%")), "12.5%");
        assert_eq!(format_number(1234.5, Some("#,##0.00")), "1,234.50");
        assert_eq!(format_number(-1234567.0, Some("#,##0")), "-1,234,567");
        assert_eq!(format_number(1.23456, Some("0.00")), "1.23");
        assert_eq!(format_number(45000.0, Some("m/d/yyyy")), "45000");
        assert_eq!(format_number(7.0, Some("[Red]\"$\"0.0")), "7.0");
    }
    use crate::model::{Row, Shape, TextRun};

    fn run(text: &str) -> TextRun {
        TextRun {
            text: text.to_string(),
            ..TextRun::default()
        }
    }

    fn para(text: &str, level: u8) -> Paragraph {
        Paragraph {
            runs: vec![run(text)],
            level,
            ..Paragraph::default()
        }
    }

    fn cell(text: &str, col_span: u32, row_span: u32, merged: bool) -> Cell {
        Cell {
            paragraphs: if text.is_empty() {
                Vec::new()
            } else {
                vec![para(text, 0)]
            },
            col_span,
            row_span,
            fill: None,
            no_fill: false,
            merged,
            mar_l: None,
            mar_r: None,
            mar_t: None,
            mar_b: None,
            anchor: None,
            borders: crate::model::CellBorders::default(),
        }
    }

    fn slide_with(shapes: Vec<Shape>, notes: Option<&str>) -> Slide {
        Slide {
            index: 0,
            shapes,
            layout_name: None,
            master_name: None,
            notes: notes.map(|s| s.to_string()),
            clr_map_ovr: None,
            background: None,
            hidden: false,
            show_master_sp: true,
            comments: Vec::new(),
        }
    }

    #[test]
    fn gfm_table_for_unmerged() {
        let table = Table {
            rect: None,
            col_widths: Vec::new(),
            table_style_id: None,
            flags: crate::model::TableFlags::default(),
            rows: vec![
                Row {
                    cells: vec![cell("A1", 1, 1, false), cell("B1", 1, 1, false)],
                    height: None,
                },
                Row {
                    cells: vec![cell("A2", 1, 1, false), cell("B2", 1, 1, false)],
                    height: None,
                },
            ],
        };
        let md = table_markdown(&table, None);
        assert!(md.contains("| A1 | B1 |"));
        assert!(md.contains("| --- | --- |"));
        assert!(md.contains("| A2 | B2 |"));
        assert!(!md.contains("<table>"));
    }

    #[test]
    fn html_table_for_merged() {
        let table = Table {
            rect: None,
            col_widths: Vec::new(),
            table_style_id: None,
            flags: crate::model::TableFlags::default(),
            rows: vec![
                Row {
                    // gridSpan=2 表头 + 一个 hMerge 延续格
                    cells: vec![cell("Header", 2, 1, false), cell("", 1, 1, true)],
                    height: None,
                },
                Row {
                    cells: vec![cell("A2", 1, 1, false), cell("B2", 1, 1, false)],
                    height: None,
                },
            ],
        };
        let md = table_markdown(&table, None);
        assert!(md.starts_with("<table>"));
        assert!(md.contains("<td colspan=\"2\">Header</td>"));
        assert!(md.contains("<td>A2</td>"));
        // 延续格不应单独输出。
        assert_eq!(md.matches("<td").count(), 3);
    }

    #[test]
    fn text_and_markdown_with_notes() {
        let title = Shape::TextBox(TextFrame {
            paragraphs: vec![para("Deck Title", 0), para("bullet", 1)],
            ..TextFrame::default()
        });
        let slide = slide_with(vec![title], Some("remember this"));
        let pres = Presentation {
            slides: vec![slide],
            slide_size: (0, 0),
            sections: Vec::new(),
            properties: Default::default(),
            first_slide_num: 1,
            diagnostics: Vec::new(),
        };

        let text = presentation_text(&pres);
        assert!(text.contains("--- slide 1 ---"));
        assert!(text.contains("Deck Title"));
        assert!(text.contains("Notes:\nremember this"));

        let md = presentation_markdown(&pres);
        assert!(md.contains("## Slide 1"));
        assert!(md.contains("### Deck Title"));
        assert!(md.contains("- bullet"));
        assert!(md.contains("> Notes:\n> remember this"));
    }

    #[test]
    fn escapes_html_in_html_table() {
        let table = Table {
            rect: None,
            col_widths: Vec::new(),
            table_style_id: None,
            flags: crate::model::TableFlags::default(),
            rows: vec![Row {
                cells: vec![cell("a<b>&c", 2, 1, false), cell("", 1, 1, true)],
                height: None,
            }],
        };
        let md = table_markdown(&table, None);
        assert!(md.contains("a&lt;b&gt;&amp;c"));
    }
}
