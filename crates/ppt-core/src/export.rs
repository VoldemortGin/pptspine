//! 结构化导出:把已解析的模型渲染成纯文本 / Markdown。
//!
//! 基于现有领域模型(无 IO / XML),逐 slide 拼接文字、表格,并可选纳入演讲者备注。
//! 设计目标:确定性、容错(空内容不产生噪声)、对合并单元格保真(Markdown 走 HTML `<table>`)。

use crate::model::{Cell, Paragraph, Presentation, Shape, Slide, Table, TextFrame};

/// 一张幻灯片的正文文字(所有文本框 / 自选图形文字 / 表格,按文档顺序;**不含备注**)。
pub fn slide_text(slide: &Slide) -> String {
    let mut blocks: Vec<String> = Vec::new();
    for sh in &slide.shapes {
        collect_shape_text(sh, &mut blocks);
    }
    blocks.join("\n")
}

/// 整份演示文稿的纯文本:各 slide 以 `--- slide N ---` 分隔(1 基序号),可附带备注。
pub fn presentation_text(pres: &Presentation) -> String {
    let mut sections: Vec<String> = Vec::new();
    for slide in &pres.slides {
        let mut sect = format!("--- slide {} ---", slide.index + 1);
        let body = slide_text(slide);
        if !body.is_empty() {
            sect.push('\n');
            sect.push_str(&body);
        }
        if let Some(notes) = notes_text(slide) {
            sect.push_str("\n\nNotes:\n");
            sect.push_str(&notes);
        }
        sections.push(sect);
    }
    sections.join("\n\n")
}

/// 整份演示文稿的 Markdown:每页一节(`## Slide N`),首个文本框作标题,表格用
/// GFM(无合并)或 HTML `<table>`(含 `gridSpan`/`rowSpan` 合并)保真,备注以引用块附后。
pub fn presentation_markdown(pres: &Presentation) -> String {
    pres.slides
        .iter()
        .map(slide_markdown)
        .collect::<Vec<_>>()
        .join("\n\n")
}

// ---- 纯文本辅助 ----------------------------------------------------------

fn collect_shape_text(shape: &Shape, out: &mut Vec<String>) {
    match shape {
        Shape::TextBox(tf) => push_frame_text(tf, out),
        Shape::Auto(a) => {
            if let Some(tf) = &a.text {
                push_frame_text(tf, out);
            }
        }
        Shape::Table(t) => {
            let s = table_text(t);
            if !s.is_empty() {
                out.push(s);
            }
        }
        Shape::Picture(_) | Shape::Connector(_) | Shape::Placeholder(_) => {}
        Shape::Group(g) => {
            for c in &g.children {
                collect_shape_text(c, out);
            }
        }
    }
}

fn push_frame_text(tf: &TextFrame, out: &mut Vec<String>) {
    let s = frame_text(tf);
    if !s.is_empty() {
        out.push(s);
    }
}

fn frame_text(tf: &TextFrame) -> String {
    tf.paragraphs
        .iter()
        .map(paragraph_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn paragraph_text(p: &Paragraph) -> String {
    p.runs.iter().map(|r| r.text.as_str()).collect()
}

fn cell_text(c: &Cell) -> String {
    c.paragraphs
        .iter()
        .map(paragraph_text)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn table_text(t: &Table) -> String {
    let mut lines: Vec<String> = Vec::new();
    for row in &t.rows {
        let cells: Vec<String> = row
            .cells
            .iter()
            .filter(|c| !c.merged) // 被合并掉的延续格不重复输出
            .map(cell_text)
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

// ---- Markdown 辅助 -------------------------------------------------------

fn slide_markdown(slide: &Slide) -> String {
    let mut out = String::new();
    out.push_str(&format!("## Slide {}", slide.index + 1));

    let mut title_done = false;
    let mut blocks: Vec<String> = Vec::new();
    for sh in &slide.shapes {
        collect_shape_markdown(sh, &mut title_done, &mut blocks);
    }
    for block in blocks {
        out.push_str("\n\n");
        out.push_str(&block);
    }

    if let Some(notes) = notes_text(slide) {
        let quoted = notes
            .lines()
            .map(|l| format!("> {l}"))
            .collect::<Vec<_>>()
            .join("\n");
        out.push_str("\n\n> Notes:\n");
        out.push_str(&quoted);
    }
    out
}

fn collect_shape_markdown(shape: &Shape, title_done: &mut bool, out: &mut Vec<String>) {
    match shape {
        Shape::TextBox(tf) => frame_markdown(tf, title_done, out),
        Shape::Auto(a) => {
            if let Some(tf) = &a.text {
                frame_markdown(tf, title_done, out);
            }
        }
        Shape::Table(t) => {
            let md = table_markdown(t);
            if !md.is_empty() {
                out.push(md);
            }
        }
        Shape::Picture(_) | Shape::Connector(_) | Shape::Placeholder(_) => {}
        Shape::Group(g) => {
            for c in &g.children {
                collect_shape_markdown(c, title_done, out);
            }
        }
    }
}

fn frame_markdown(tf: &TextFrame, title_done: &mut bool, out: &mut Vec<String>) {
    let paras: Vec<(u8, String)> = tf
        .paragraphs
        .iter()
        .map(|p| (p.level, paragraph_text(p)))
        .filter(|(_, t)| !t.trim().is_empty())
        .collect();
    if paras.is_empty() {
        return;
    }
    let mut iter = paras.into_iter();
    if !*title_done {
        // 首个非空文本框的第一段当 slide 标题。
        let (_, title) = iter.next().expect("paras non-empty");
        out.push(format!("### {title}"));
        *title_done = true;
    }
    for (level, text) in iter {
        if level == 0 {
            out.push(text);
        } else {
            // 缩进的项目符号(层级 >= 1)。
            let indent = "  ".repeat((level as usize).saturating_sub(1));
            out.push(format!("{indent}- {text}"));
        }
    }
}

fn table_markdown(t: &Table) -> String {
    if t.rows.is_empty() {
        return String::new();
    }
    let has_merge = t
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .any(|c| c.col_span > 1 || c.row_span > 1 || c.merged);
    if has_merge {
        table_html(t)
    } else {
        table_gfm(t)
    }
}

fn table_gfm(t: &Table) -> String {
    let cols = t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    if cols == 0 {
        return String::new();
    }
    let mut lines: Vec<String> = Vec::new();
    for (i, row) in t.rows.iter().enumerate() {
        let mut cells: Vec<String> = row
            .cells
            .iter()
            .map(|c| escape_pipe(&cell_text(c)))
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

fn table_html(t: &Table) -> String {
    let mut s = String::from("<table>");
    for row in &t.rows {
        s.push_str("\n  <tr>");
        for c in &row.cells {
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
                escape_html(&cell_text(c))
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
    use crate::model::{Row, TextRun};

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
        }
    }

    #[test]
    fn gfm_table_for_unmerged() {
        let table = Table {
            rect: None,
            col_widths: Vec::new(),
            table_style_id: None,
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
        let md = table_markdown(&table);
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
        let md = table_markdown(&table);
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
            rows: vec![Row {
                cells: vec![cell("a<b>&c", 2, 1, false), cell("", 1, 1, true)],
                height: None,
            }],
        };
        let md = table_markdown(&table);
        assert!(md.contains("a&lt;b&gt;&amp;c"));
    }
}
