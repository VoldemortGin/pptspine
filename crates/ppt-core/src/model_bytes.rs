//! 模型字节估算:按"结构体大小 × 个数 + 字符串字节数"估算解析结果占用的内存主项,供
//! `ZipLimits::max_model_bytes`(模型字节预算)的验收与调用方自查。
//!
//! 计入:每个形状 / 段落 / run / 表格行 / 单元格 / 批注 / 图表系列的结构体大小,run 文字、
//! 超链接目标、图片替代文本 / 名称 / 标题、图表类别名 / 系列名 / 数值、SmartArt 退回文字、
//! 备注、批注正文 / 时间 / 作者。**共享**的值(`Arc`:超链接目标、批注作者、图表)按指针去重,
//! 只在第一次遇到时计入——与解析侧"共享的只在首次创建时扣预算"同一口径。
//!
//! 不计:分配器开销与 `Vec` 增长余量、样式 / 颜色等小字段——所以这是**主项估算**(下界性质),
//! 解析侧按实际产生的每个字符串记账,总是不小于这里的估算。

use std::collections::BTreeSet;
use std::mem::size_of;
use std::sync::Arc;

use crate::model::{Chart, Comment, Paragraph, Presentation, Shape, Slide, TextFrame, TextRun};

/// 带共享去重状态的估算器(同一个 `Arc` 只计一次)。
#[derive(Debug, Default)]
pub struct Estimator {
    seen: BTreeSet<usize>,
}

impl Estimator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 一个共享值第一次出现返回 `true`(按 `Arc` 指针去重)。
    fn first<T: ?Sized>(&mut self, a: &Arc<T>) -> bool {
        self.seen.insert(Arc::as_ptr(a).cast::<u8>() as usize)
    }

    fn shared_str(&mut self, s: Option<&Arc<str>>) -> usize {
        match s {
            Some(a) if self.first(a) => a.len(),
            _ => 0,
        }
    }

    /// 整份演示文稿(幻灯片 + 形状 + 备注 + 批注)。
    pub fn presentation(&mut self, p: &Presentation) -> usize {
        p.slides.iter().map(|s| self.slide(s)).sum()
    }

    /// 一张幻灯片。
    pub fn slide(&mut self, s: &Slide) -> usize {
        size_of::<Slide>()
            + self.shapes(&s.shapes)
            + s.notes.as_ref().map_or(0, String::len)
            + s.comments.iter().map(|c| self.comment(c)).sum::<usize>()
    }

    /// 一条批注(含回复)。
    pub fn comment(&mut self, c: &Comment) -> usize {
        size_of::<Comment>()
            + c.text.as_ref().map_or(0, String::len)
            + c.datetime.as_ref().map_or(0, String::len)
            + self.shared_str(c.author.as_ref())
            + self.shared_str(c.initials.as_ref())
            + c.replies.iter().map(|r| self.comment(r)).sum::<usize>()
    }

    /// 一组形状(含组合内后代)。
    pub fn shapes(&mut self, shapes: &[Shape]) -> usize {
        shapes.iter().map(|s| self.shape(s)).sum()
    }

    fn shape(&mut self, s: &Shape) -> usize {
        let opt = |v: &Option<String>| v.as_ref().map_or(0, String::len);
        size_of::<Shape>()
            + match s {
                Shape::TextBox(t) => self.frame(t),
                Shape::Auto(a) => a
                    .text
                    .as_deref()
                    .map_or(0, |t| size_of::<TextFrame>() + self.frame(t)),
                Shape::Table(t) => {
                    t.col_widths.len() * size_of::<crate::geom::Emu>()
                        + t.rows
                            .iter()
                            .map(|r| {
                                size_of::<crate::model::Row>()
                                    + r.cells
                                        .iter()
                                        .map(|c| {
                                            size_of::<crate::model::Cell>()
                                                + self.paragraphs(&c.paragraphs)
                                        })
                                        .sum::<usize>()
                            })
                            .sum::<usize>()
                }
                Shape::Picture(p) => opt(&p.alt_text) + opt(&p.name) + opt(&p.title),
                Shape::Group(g) => self.shapes(&g.children),
                Shape::Placeholder(p) => {
                    p.diagram_text
                        .iter()
                        .map(|t| size_of::<String>() + t.len())
                        .sum::<usize>()
                        + p.chart.as_ref().map_or(0, |c| self.chart(c))
                }
                Shape::Connector(_) => 0,
            }
    }

    fn frame(&mut self, t: &TextFrame) -> usize {
        self.paragraphs(&t.paragraphs)
    }

    fn paragraphs(&mut self, paras: &[Paragraph]) -> usize {
        paras
            .iter()
            .map(|p| size_of::<Paragraph>() + p.runs.iter().map(|r| self.run(r)).sum::<usize>())
            .sum()
    }

    fn run(&mut self, r: &TextRun) -> usize {
        size_of::<TextRun>()
            + r.text.len()
            + self.shared_str(r.hyperlink.as_ref().and_then(|h| h.url.as_ref()))
    }

    /// 一张共享图表(同一 `Arc` 只计一次)。
    pub fn chart(&mut self, c: &Arc<Chart>) -> usize {
        if !self.first(c) {
            return 0;
        }
        chart_bytes(c)
    }
}

/// 一张图表数据的字节估算(不去重)。
#[must_use]
pub fn chart_bytes(c: &Chart) -> usize {
    size_of::<Chart>()
        + c.title.as_ref().map_or(0, String::len)
        + c.categories
            .iter()
            .map(|s| size_of::<String>() + s.len())
            .sum::<usize>()
        + c.series
            .iter()
            .map(|s| {
                size_of::<crate::model::ChartSeries>()
                    + s.name.as_ref().map_or(0, String::len)
                    + s.values.len() * size_of::<Option<f64>>()
            })
            .sum::<usize>()
}

/// 整份演示文稿的模型字节估算(共享值只计一次)。
#[must_use]
pub fn presentation_bytes(p: &Presentation) -> usize {
    Estimator::new().presentation(p)
}

/// 一组形状的模型字节估算(独立去重状态)。
#[must_use]
pub fn shapes_bytes(shapes: &[Shape]) -> usize {
    Estimator::new().shapes(shapes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GraphicPlaceholder, Hyperlink};

    fn link_run(url: &Arc<str>) -> TextRun {
        TextRun {
            text: "x".into(),
            hyperlink: Some(Hyperlink {
                url: Some(Arc::clone(url)),
                ..Hyperlink::default()
            }),
            ..TextRun::default()
        }
    }

    #[test]
    fn shared_values_are_counted_once() {
        let url: Arc<str> = Arc::from("u".repeat(10_000));
        let frame = |n: usize| {
            Shape::TextBox(TextFrame {
                paragraphs: vec![Paragraph {
                    runs: (0..n).map(|_| link_run(&url)).collect(),
                    ..Paragraph::default()
                }],
                ..TextFrame::default()
            })
        };
        let one = shapes_bytes(&[frame(1)]);
        let many = shapes_bytes(&[frame(100)]);
        // 100 个 run 共享同一个 URL:多出来的只是 99 个 run 的结构体 + 文字,不是 99 份 URL。
        assert_eq!(many - one, 99 * (size_of::<TextRun>() + 1));
        assert!(one > 10_000);
    }

    #[test]
    fn shared_chart_is_counted_once_across_frames() {
        let chart = Arc::new(Chart {
            kind: crate::model::ChartKind::Bar,
            title: None,
            categories: vec!["c".repeat(5_000)],
            series: Vec::new(),
            bar_dir: None,
            grouping: None,
            three_d: false,
            combo: false,
            of_pie: false,
            warnings: Vec::new(),
        });
        let frame = || {
            Shape::Placeholder(GraphicPlaceholder {
                rect: None,
                kind: None,
                chart_rel_id: None,
                chart: Some(Arc::clone(&chart)),
                diagram_rel_id: None,
                diagram_text: Vec::new(),
            })
        };
        let one = shapes_bytes(&[frame()]);
        let ten = shapes_bytes(&vec![frame(); 10]);
        assert_eq!(ten - one, 9 * size_of::<Shape>());
    }
}
