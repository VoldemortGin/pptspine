//! 解析批注部件与作者部件。
//!
//! - 旧式:`ppt/comments/commentN.xml`(`p:cmLst > p:cm`,`@authorId` 整数 id、`@dt`、
//!   `p:pos@x/@y`、`p:text`),作者在 `ppt/commentAuthors.xml`(`p:cmAuthor`)。
//! - 新式线程批注:`p188:cmLst > p188:cm`(`@authorId` GUID、`@created`、`p188:txBody`、
//!   `p188:replyLst > p188:reply`),作者在 `ppt/authors.xml`(`p188:author`)。
//!
//! 两种结构只在元素 / 属性名上略有不同,按本地名统一处理。作者 / 正文是隐私数据,这里
//! 只构造模型,不写任何告警 / 日志。容错:未知元素跳过、畸形输入返回已得部分、绝不 panic。

use std::collections::BTreeMap;
use std::sync::Arc;

use ppt_core::model::Comment;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::budget::{fit_string, MAX_LABEL_BYTES};
use super::slide::parse_txbody;
use super::{attr_of, local_name, read_text, skip_element};

// 测试用:本线程内批注部件解析出的条目数(批注 + 回复),断言"解析量按剩余额度封顶"。
#[cfg(test)]
thread_local! {
    pub(crate) static PARSED_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// 作者表条目(`@name` / `@initials`)。共享所有权:同一作者的每条批注引用同一份字符串
/// (一个很长的作者名 × 海量批注不会被复制成 N 份);长度截到 [`MAX_LABEL_BYTES`]。
#[derive(Debug, Clone, Default)]
pub struct Author {
    pub name: Option<Arc<str>>,
    pub initials: Option<Arc<str>>,
}

/// 作者名 / 缩写:截到标签上限后转成共享字符串。
fn label(v: Option<String>) -> Option<Arc<str>> {
    v.map(|mut s| {
        fit_string(&mut s, MAX_LABEL_BYTES);
        Arc::from(s)
    })
}

/// 解析作者部件(`p:cmAuthorLst` 或 `p188:authorLst`)→ `id -> Author`。
pub fn parse_authors(xml: &str) -> BTreeMap<String, Author> {
    let mut map = BTreeMap::new();
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) => {
                if matches!(local_name(e.name().as_ref()), b"cmAuthor" | b"author") {
                    if let Some(id) = attr_of(&e, b"id") {
                        map.insert(
                            id,
                            Author {
                                name: label(attr_of(&e, b"name")),
                                initials: label(attr_of(&e, b"initials")),
                            },
                        );
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    map
}

/// 一份批注部件的解析结果。
pub struct ParsedComments {
    /// 批注列表(文档顺序)。
    pub comments: Vec<Comment>,
    /// 因 `max` 上限提前停止(部件里还有没读的批注 / 回复)。
    pub truncated: bool,
    /// 解析正文时因嵌套超限被跳过的子树数(诊断用)。
    pub nesting_skipped: usize,
}

/// 批注 / 回复的剩余条数预算;耗尽后再遇到条目就记 `truncated`。
struct Budget {
    left: usize,
    truncated: bool,
}

impl Budget {
    /// 取走一条;预算已空则标记截断并返回 `false`。
    fn take(&mut self) -> bool {
        // 条数额度 + 模型字节预算(结构体本身;作者是共享的,不另计)。
        if self.left == 0 || !super::budget::take_item(std::mem::size_of::<Comment>()) {
            self.truncated = true;
            return false;
        }
        self.left -= 1;
        #[cfg(test)]
        PARSED_ENTRIES.with(|c| c.set(c.get() + 1));
        true
    }
}

/// 批注 / 回复总条数(含回复)。
pub fn count_comments(cs: &[Comment]) -> usize {
    cs.iter().map(|c| 1 + c.replies.len()).sum()
}

/// 解析一份批注部件 → 批注列表(文档顺序)。批注与回复合计最多读 `max` 条,
/// 之后立即停止(防一个部件里的海量批注先被全量读进内存)。
pub fn parse_comments(xml: &str, authors: &BTreeMap<String, Author>, max: usize) -> ParsedComments {
    let mut out = Vec::new();
    super::slide::reset_nest_skipped();
    let mut budget = Budget {
        left: max,
        truncated: false,
    };
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"cm" if !budget.take() => break,
                    b"cm" => out.push(parse_cm(&mut reader, &e, authors, true, &mut budget)),
                    // 容器(cmLst)继续下钻;其余整体跳过。
                    b"cmLst" => {}
                    _ => skip_element(&mut reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"cm" {
                    if !budget.take() {
                        break;
                    }
                    out.push(comment_head(&e, authors));
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    ParsedComments {
        comments: out,
        truncated: budget.truncated,
        nesting_skipped: super::slide::take_nest_skipped(),
    }
}

/// 批注 / 回复的头部属性(作者经表查找;`@dt` / `@created` 取时间)。
fn comment_head(e: &BytesStart, authors: &BTreeMap<String, Author>) -> Comment {
    let author = attr_of(e, b"authorId").and_then(|id| authors.get(&id));
    Comment {
        author: author.and_then(|a| a.name.clone()),
        initials: author.and_then(|a| a.initials.clone()),
        datetime: attr_of(e, b"dt").or_else(|| attr_of(e, b"created")),
        ..Comment::default()
    }
}

/// 解析一个 `cm` / `reply`(已消费起始标签)。`allow_replies` 为 `false`(回复内部)时
/// 忽略 `replyLst`,保证递归深度有界。
fn parse_cm<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    start: &BytesStart,
    authors: &BTreeMap<String, Author>,
    allow_replies: bool,
    budget: &mut Budget,
) -> Comment {
    let mut c = comment_head(start, authors);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"text" => c.text = Some(read_text(reader)),
                    b"txBody" => {
                        let paras: Vec<String> = parse_txbody(reader)
                            .paragraphs
                            .iter()
                            .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
                            .collect();
                        c.text = Some(paras.join("\n"));
                    }
                    b"pos" => {
                        c.position = pos_of(&e);
                        skip_element(reader, &name);
                    }
                    b"replyLst" if allow_replies => parse_replies(reader, authors, &mut c, budget),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"pos" {
                    c.position = pos_of(&e);
                }
            }
            Ok(Event::End(_) | Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    c
}

/// `replyLst` 内逐条 `reply`(已消费 `<replyLst>` 起始标签)。
fn parse_replies<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    authors: &BTreeMap<String, Author>,
    parent: &mut Comment,
    budget: &mut Budget,
) {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"reply" {
                    if !budget.take() {
                        skip_element(reader, &name);
                        break;
                    }
                    parent
                        .replies
                        .push(parse_cm(reader, &e, authors, false, budget));
                } else {
                    skip_element(reader, &name);
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"reply" {
                    if !budget.take() {
                        break;
                    }
                    parent.replies.push(comment_head(&e, authors));
                }
            }
            Ok(Event::End(_) | Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
}

/// `p:pos@x` / `@y`(两者齐全且可解析才算有位置)。
fn pos_of(e: &BytesStart) -> Option<(i64, i64)> {
    let x = attr_of(e, b"x")?.trim().parse().ok()?;
    let y = attr_of(e, b"y")?.trim().parse().ok()?;
    Some((x, y))
}
