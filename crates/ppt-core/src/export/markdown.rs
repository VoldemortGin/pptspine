//! 语义 Markdown 导出(对标 MarkItDown / docling 的 pptx 抽取):
//!
//! - 每页一节 `## Slide N`;标题取 `title` / `ctrTitle` 占位符文字(`### …`),无标题占位符时
//!   回退到"阅读顺序里首个非空文本框的首段";`subTitle` 占位符按普通段落输出;
//! - 列表标记来自继承链解析后的段落项目符号(`buChar` → `- `,`buAutoNum` → `N. `,
//!   `buNone` / 未设 → 无标记),按层级缩进两个空格;无解析信息时退回段落直接格式;
//! - 图片输出 `![alt](media)`(alt = `cNvPr@descr`,缺省 `@name`);
//! - 外部超链接输出 `[text](url)`(run 级优先,形状级兜底);内部跳转按纯文本;
//! - 表格沿用 GFM / HTML `<table>`(合并单元格)保真;备注以引用块附后;
//! - 图表(缓存数据)输出 `#### Chart: <title>` + GFM 表格(对标 MarkItDown)。

use crate::model::{Chart, Hyperlink, Paragraph, Picture, Presentation, Shape, Slide, TextFrame};
use crate::resolved::{
    ResolvedBullet, ResolvedParagraph, ResolvedPresentation, ResolvedShape, ResolvedSlide,
};
use crate::style::Bullet;

use super::reading_order::FlatShape;
use super::view::{exported_slides, ordered_shapes, placeholder_kind, ExportOptions};
use super::{chart_label, chart_table, escape_pipe, notes_text, paragraph_text, table_markdown};

/// 整份演示文稿的语义 Markdown。`resolved` 提供继承链信息(占位符几何 / 项目符号);
/// 可为 `None`(退回直接格式)。
pub fn presentation_markdown_with(
    pres: &Presentation,
    resolved: Option<&ResolvedPresentation>,
    opts: &ExportOptions,
) -> String {
    exported_slides(pres, resolved, opts)
        .map(|(slide, rs)| slide_markdown(slide, rs, pres, opts))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn slide_markdown(
    slide: &Slide,
    resolved: Option<&ResolvedSlide>,
    pres: &Presentation,
    opts: &ExportOptions,
) -> String {
    let shapes = ordered_shapes(slide, resolved, pres.slide_size, opts.order);
    let mut out = format!("## Slide {}", slide.index + 1);

    // 标题:首个有文字的 title / ctrTitle 占位符;没有则回退到首个非空文本框的首段。
    let title_at = shapes.iter().position(|f| {
        matches!(placeholder_kind(f.shape), Some("title" | "ctrTitle"))
            && text_frame(f.shape).is_some_and(|tf| !heading_text(&tf.paragraphs).is_empty())
    });
    let mut skip_first_para_of: Option<usize> = None;
    let title = match title_at {
        Some(i) => text_frame(shapes[i].shape).map(|tf| heading_text(&tf.paragraphs)),
        // 回退策略(历史行为):该框首个非空段落作标题,其余段落照常输出。
        None => shapes.iter().enumerate().find_map(|(i, f)| {
            let tf = text_frame(f.shape)?;
            let p = tf
                .paragraphs
                .iter()
                .find(|p| !paragraph_text(p).trim().is_empty())?;
            skip_first_para_of = Some(i);
            Some(one_line(&paragraph_text(p)))
        }),
    };
    if let Some(t) = title {
        out.push_str("\n\n### ");
        out.push_str(&t);
    }

    let mut blocks: Vec<String> = Vec::new();
    for (i, f) in shapes.iter().enumerate() {
        if Some(i) == title_at {
            continue;
        }
        shape_blocks(f, skip_first_para_of == Some(i), &mut blocks);
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

/// 形状的文字体(文本框 / 自选图形内文字)。
fn text_frame(shape: &Shape) -> Option<&TextFrame> {
    match shape {
        Shape::TextBox(tf) => Some(tf),
        Shape::Auto(a) => a.text.as_deref(),
        _ => None,
    }
}

/// 解析后的段落(与原始段落一一对应时才可用)。
fn resolved_paragraphs<'a>(f: &FlatShape<'a>, raw_len: usize) -> Option<&'a [ResolvedParagraph]> {
    let paras = match f.resolved? {
        ResolvedShape::TextBox(t) => t.paragraphs.as_slice(),
        ResolvedShape::Auto(a) => a.text.as_ref()?.paragraphs.as_slice(),
        _ => return None,
    };
    (paras.len() == raw_len).then_some(paras)
}

fn shape_blocks(f: &FlatShape, skip_first_para: bool, out: &mut Vec<String>) {
    match f.shape {
        Shape::TextBox(_) | Shape::Auto(_) => {
            if let Some(tf) = text_frame(f.shape) {
                frame_blocks(f, tf, skip_first_para, out);
            }
        }
        Shape::Table(t) => {
            let md = table_markdown(t);
            if !md.is_empty() {
                out.push(md);
            }
        }
        Shape::Picture(p) => out.push(picture_markdown(p)),
        Shape::Placeholder(p) => {
            if let Some(c) = &p.chart {
                out.push(chart_markdown(c));
            }
        }
        Shape::Connector(_) | Shape::Group(_) => {}
    }
}

/// 图表:`#### Chart: <title>`(无标题 `#### Chart (<kind>)`)+ GFM 表格(首列类别,各系列一列)。
fn chart_markdown(c: &Chart) -> String {
    let mut out = format!("#### {}", chart_label(c));
    if let Some((header, rows)) = chart_table(c) {
        let line = |cells: &[String]| {
            let cells: Vec<String> = cells.iter().map(|s| escape_pipe(s)).collect();
            format!("| {} |", cells.join(" | "))
        };
        out.push_str("\n\n");
        out.push_str(&line(&header));
        out.push_str(&format!("\n| {} |", vec!["---"; header.len()].join(" | ")));
        for r in &rows {
            out.push('\n');
            out.push_str(&line(r));
        }
    }
    out
}

/// 段落的列表标记。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    None,
    Bullet,
    Number { start: i64 },
}

fn marker_of(p: &Paragraph, resolved: Option<&ResolvedParagraph>, plain: bool) -> Marker {
    if plain {
        return Marker::None;
    }
    let num = |start_at: Option<i32>| Marker::Number {
        start: i64::from(start_at.unwrap_or(1)),
    };
    if let Some(rp) = resolved {
        return match &rp.bullet {
            ResolvedBullet::None => Marker::None,
            ResolvedBullet::Char { .. } => Marker::Bullet,
            ResolvedBullet::AutoNum { start_at, .. } => num(*start_at),
        };
    }
    // 无继承链信息:段落直接格式;再无则沿用历史启发(层级 ≥ 1 视作项目符号)。
    match &p.props.bullet {
        Some(Bullet::None) => Marker::None,
        Some(Bullet::Char(_)) => Marker::Bullet,
        Some(Bullet::AutoNum { start_at, .. }) => num(*start_at),
        None if p.level >= 1 => Marker::Bullet,
        None => Marker::None,
    }
}

fn frame_blocks(f: &FlatShape, tf: &TextFrame, skip_first_para: bool, out: &mut Vec<String>) {
    let resolved = resolved_paragraphs(f, tf.paragraphs.len());
    // subTitle 占位符按普通段落(不加列表标记)。
    let plain = placeholder_kind(f.shape) == Some("subTitle");
    let shape_link = tf.hyperlink.as_ref().or(match f.shape {
        Shape::Auto(a) => a.hyperlink.as_ref(),
        _ => None,
    });

    // 每层自动编号的当前值;同层非编号段落 / 更浅层段落会重置更深层计数。
    let mut counters: [Option<i64>; 9] = [None; 9];
    let mut skipped = !skip_first_para;
    let mut list_block: Option<String> = None;
    for (i, p) in tf.paragraphs.iter().enumerate() {
        let text = paragraph_markdown(p, shape_link);
        if text.trim().is_empty() {
            continue;
        }
        if !skipped {
            skipped = true; // 回退标题已取走此段。
            continue;
        }
        let level = usize::from(p.level).min(8);
        let marker = marker_of(p, resolved.map(|r| &r[i]), plain);
        for c in counters.iter_mut().skip(level + 1) {
            *c = None;
        }
        let line = match marker {
            Marker::None => {
                counters[level] = None;
                if let Some(b) = list_block.take() {
                    out.push(b);
                }
                out.push(text);
                continue;
            }
            Marker::Bullet => {
                counters[level] = None;
                format!("{}- {text}", "  ".repeat(level))
            }
            Marker::Number { start } => {
                let n = counters[level].map_or(start, |c| c.saturating_add(1));
                counters[level] = Some(n);
                format!("{}{n}. {text}", "  ".repeat(level))
            }
        };
        match list_block.as_mut() {
            Some(b) => {
                b.push('\n');
                b.push_str(&line);
            }
            None => list_block = Some(line),
        }
    }
    if let Some(b) = list_block {
        out.push(b);
    }
}

/// 段落 → Markdown 行内文本:外链 run 折成 `[text](url)`(相邻同链 run 合并);
/// 形状级外链作为无自身链接 run 的兜底;内部跳转按纯文本。
fn paragraph_markdown(p: &Paragraph, shape_link: Option<&Hyperlink>) -> String {
    let mut out = String::new();
    let mut pending: Option<(&str, String)> = None; // (url, 累积文字)
    let flush = |pending: &mut Option<(&str, String)>, out: &mut String| {
        if let Some((url, text)) = pending.take() {
            if text.trim().is_empty() {
                out.push_str(&text);
            } else {
                out.push_str(&format!("[{}]({})", escape_brackets(&text), link_dest(url)));
            }
        }
    };
    for run in &p.runs {
        let url = run
            .hyperlink
            .as_ref()
            .or(shape_link)
            .and_then(|h| h.url.as_deref());
        match url {
            Some(u) => {
                if pending.as_ref().is_some_and(|(pu, _)| *pu != u) {
                    flush(&mut pending, &mut out);
                }
                pending
                    .get_or_insert_with(|| (u, String::new()))
                    .1
                    .push_str(&run.text);
            }
            None => {
                flush(&mut pending, &mut out);
                out.push_str(&run.text);
            }
        }
    }
    flush(&mut pending, &mut out);
    out
}

fn picture_markdown(p: &Picture) -> String {
    let alt = p
        .alt_text
        .as_deref()
        .or(p.name.as_deref())
        .map(|s| escape_brackets(&one_line(s)))
        .unwrap_or_default();
    let target = p.media_name.as_deref().unwrap_or("#");
    let img = format!("![{alt}]({})", link_dest(target));
    match p.hyperlink.as_ref().and_then(|h| h.url.as_deref()) {
        Some(url) => format!("[{img}]({})", link_dest(url)),
        None => img,
    }
}

/// 标题文字:各非空段落拼成一行(段内换行 / 段落间以空格连接)。
fn heading_text(paras: &[Paragraph]) -> String {
    paras
        .iter()
        .map(|p| one_line(&paragraph_text(p)))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 折成单行(换行 → 空格,去首尾空白)。
fn one_line(s: &str) -> String {
    s.split(['\n', '\r'])
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 转义链接 / 替代文本里的方括号,避免破坏 `[…](…)` 结构。
fn escape_brackets(s: &str) -> String {
    s.replace('[', "\\[").replace(']', "\\]")
}

/// 链接目标:含空白或括号时用 `<…>` 包起来(CommonMark 尖括号目标)。
fn link_dest(url: &str) -> String {
    if url.contains(|c: char| c.is_whitespace() || c == '(' || c == ')') {
        format!("<{}>", url.replace('<', "%3C").replace('>', "%3E"))
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;
    use crate::model::TextRun;
    use crate::style::PlaceholderRef;

    fn run(text: &str) -> TextRun {
        TextRun {
            text: text.to_string(),
            ..TextRun::default()
        }
    }

    fn linked(text: &str, url: &str) -> TextRun {
        TextRun {
            hyperlink: Some(Hyperlink {
                url: Some(url.to_string()),
                ..Hyperlink::default()
            }),
            ..run(text)
        }
    }

    fn para(runs: Vec<TextRun>, bullet: Option<Bullet>, level: u8) -> Paragraph {
        let mut p = Paragraph {
            runs,
            level,
            ..Paragraph::default()
        };
        p.props.bullet = bullet;
        p
    }

    fn frame(y: i64, ph: Option<&str>, paragraphs: Vec<Paragraph>) -> Shape {
        Shape::TextBox(TextFrame {
            rect: Some(Rect::new(0, y, 1_000_000, 500_000)),
            placeholder: ph.map(|k| PlaceholderRef {
                kind: Some(k.to_string()),
                idx: None,
            }),
            paragraphs,
            ..TextFrame::default()
        })
    }

    fn deck(shapes: Vec<Shape>) -> Presentation {
        Presentation {
            slides: vec![Slide {
                index: 0,
                shapes,
                layout_name: None,
                master_name: None,
                notes: None,
                clr_map_ovr: None,
                background: None,
                hidden: false,
                show_master_sp: true,
            }],
            slide_size: (9_144_000, 6_858_000),
            sections: Vec::new(),
            properties: Default::default(),
        }
    }

    #[test]
    fn title_placeholder_wins_over_first_textbox() {
        let pres = deck(vec![
            frame(0, None, vec![para(vec![run("Banner")], None, 0)]),
            frame(
                800_000,
                Some("title"),
                vec![para(vec![run("Real Title")], None, 0)],
            ),
            frame(
                1_600_000,
                Some("subTitle"),
                vec![para(vec![run("Sub")], None, 0)],
            ),
        ]);
        let md = presentation_markdown_with(&pres, None, &ExportOptions::default());
        assert!(
            md.starts_with("## Slide 1\n\n### Real Title\n\nBanner\n\nSub"),
            "{md}"
        );
    }

    #[test]
    fn bullets_and_numbering_from_paragraph_props() {
        let num = || {
            Some(Bullet::AutoNum {
                scheme: Some("arabicPeriod".into()),
                start_at: None,
            })
        };
        let pres = deck(vec![frame(
            0,
            Some("body"),
            vec![
                para(vec![run("Heading")], None, 0),
                para(vec![run("one")], num(), 0),
                para(vec![run("two")], num(), 0),
                para(vec![run("dot")], Some(Bullet::Char("•".into())), 1),
                para(vec![run("three")], num(), 0),
                para(vec![run("plain")], Some(Bullet::None), 0),
            ],
        )]);
        let md = presentation_markdown_with(&pres, None, &ExportOptions::default());
        // 无标题占位符 → 首段回退作标题;其余按标记输出。
        assert!(
            md.contains("### Heading\n\n1. one\n2. two\n  - dot\n3. three\n\nplain"),
            "{md}"
        );
    }

    #[test]
    fn external_links_and_pictures() {
        let pic = Shape::Picture(Picture {
            rect: Some(Rect::new(0, 2_000_000, 100, 100)),
            xfrm: Default::default(),
            rel_id: "rId2".into(),
            media_name: Some("image1.png".into()),
            image_bytes_len: 0,
            src_rect: None,
            fill_rect: None,
            placeholder: None,
            name: Some("Picture 3".into()),
            alt_text: None,
            title: None,
            hyperlink: None,
        });
        let pres = deck(vec![
            frame(0, Some("title"), vec![para(vec![run("T")], None, 0)]),
            frame(
                1_000_000,
                None,
                vec![para(
                    vec![
                        run("see "),
                        linked("the ", "https://e.x/a"),
                        linked("docs", "https://e.x/a"),
                        run(" now"),
                    ],
                    None,
                    0,
                )],
            ),
            pic,
        ]);
        let md = presentation_markdown_with(&pres, None, &ExportOptions::default());
        assert!(md.contains("see [the docs](https://e.x/a) now"), "{md}");
        assert!(md.contains("![Picture 3](image1.png)"), "{md}");
    }
}
