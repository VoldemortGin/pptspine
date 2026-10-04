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

use ppt_core::model::Comment;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::slide::parse_txbody;
use super::{attr_of, local_name, read_text, skip_element};

/// 作者表条目(`@name` / `@initials`)。
#[derive(Debug, Clone, Default)]
pub struct Author {
    pub name: Option<String>,
    pub initials: Option<String>,
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
                                name: attr_of(&e, b"name"),
                                initials: attr_of(&e, b"initials"),
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

/// 解析一份批注部件 → 批注列表(文档顺序)。
pub fn parse_comments(xml: &str, authors: &BTreeMap<String, Author>) -> Vec<Comment> {
    let mut out = Vec::new();
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"cm" => out.push(parse_cm(&mut reader, &e, authors, true)),
                    // 容器(cmLst)继续下钻;其余整体跳过。
                    b"cmLst" => {}
                    _ => skip_element(&mut reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"cm" {
                    out.push(comment_head(&e, authors));
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
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
                    b"replyLst" if allow_replies => parse_replies(reader, authors, &mut c),
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
) {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"reply" {
                    parent.replies.push(parse_cm(reader, &e, authors, false));
                } else {
                    skip_element(reader, &name);
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"reply" {
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
