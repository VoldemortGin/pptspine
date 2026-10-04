//! quick-xml walker —— 按职责拆分:
//! - [`presentation`]:解析 `presentation.xml`(画布尺寸 + 幻灯片顺序)。
//! - [`slide`]:解析单张幻灯片 -> `Vec<Shape>`。
//! - [`chart`]:解析图表部件 `c:chartSpace` -> `Chart`(只读缓存数据)。
//! - [`comments`]:解析批注部件(旧式 `p:cmLst` / 新式 `p188:cmLst`)与作者部件。
//! - [`diagram`]:解析 SmartArt data 部件 `dgm:dataModel` -> 内容点文字 + drawing 关系 id。
//! - [`table_style`]:解析 `ppt/tableStyles.xml` -> `styleId -> TableStyle`。
//!
//! 本模块根放**关系(`.rels`)解析**这类被多处复用的小工具。所有 walker 都遵循家族约定:
//! 未知元素跳过、缺失属性 → `None`、**绝不 panic**。

pub(crate) mod budget;
pub mod chart;
pub mod comments;
mod custgeom;
pub mod diagram;
pub mod doc_props;
pub mod notes;
pub mod presentation;
pub mod slide;
pub mod table_style;
pub mod text_style;
pub mod theme;

use std::collections::BTreeMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

/// 一个 OOXML 关系条目(`<Relationship Id="rIdN" Type="..." Target="..."/>`)。
#[derive(Debug, Clone)]
pub struct Relationship {
    #[allow(dead_code)] // 关系 Id(rIdN)——保留为完整 API,暂未被内部消费
    pub id: String,
    pub rel_type: String,
    pub target: String,
    /// 外部目标(`TargetMode="External"`,或 `Target` 带 URI scheme 如 `https:` / `mailto:`):
    /// 不是包内部件,不参与"悬空关系"诊断。
    pub external: bool,
}

/// 解析一份 `.rels` XML,得到 `rId -> Relationship` 映射。容错:解析出错则返回已得部分。
pub fn parse_rels(xml: &str) -> BTreeMap<String, Relationship> {
    let mut map = BTreeMap::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == b"Relationship" {
                    let mut id = String::new();
                    let mut rel_type = String::new();
                    let mut target = String::new();
                    let mut external = false;
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"Id" => id = attr_string(&attr),
                            b"Type" => rel_type = attr_string(&attr),
                            b"Target" => target = attr_string(&attr),
                            b"TargetMode" => {
                                external = attr_string(&attr).eq_ignore_ascii_case("external");
                            }
                            _ => {}
                        }
                    }
                    external |= has_uri_scheme(&target);
                    if !id.is_empty() {
                        map.insert(
                            id.clone(),
                            Relationship {
                                id,
                                rel_type,
                                target,
                                external,
                            },
                        );
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    map
}

/// `Target` 是否以 URI scheme 开头(`https:` / `mailto:` …;盘符 `C:` 单字母不算)。
fn has_uri_scheme(target: &str) -> bool {
    target.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() > 1
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// XML 是否良构且标签配对闭合(只计深度,不递归,深嵌套安全)。不良构时返回解析停止处的字节偏移:
/// 读取错误(标签错配 / 属性中途截断 …),或 EOF 时仍有未闭合标签。所有 walker 遇到这两种情形
/// 都是静默 `break`,所以这是"内容被截断"的唯一判据。
pub fn check_well_formed(xml_text: &str) -> Result<(), usize> {
    let mut reader = Reader::from_str(xml_text);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(_)) => depth += 1,
            Ok(Event::End(_)) => depth = depth.saturating_sub(1),
            Ok(Event::Eof) if depth == 0 => return Ok(()),
            Ok(Event::Eof) | Err(_) => return Err(reader.buffer_position() as usize),
            _ => {}
        }
        buf.clear();
    }
}

/// 在一份 `.rels` 里找到第一个 `Type` 包含 `kind` 子串的关系,返回其相对**源部件**
/// `source_part`(该 `.rels` 所属部件)解析出的包内路径。
/// 例如 `kind = "slideLayout"`、`kind = "slideMaster"`。
pub fn first_rel_target_with(rels_xml: &str, source_part: &str, kind: &str) -> Option<String> {
    let rels = parse_rels(rels_xml);
    rels.values()
        .find(|r| r.rel_type.contains(kind))
        .map(|r| crate::links::resolve_part_path(source_part, &r.target))
}

/// 把关系 `Target` 规范化为 `ppt/` 下的部件路径(去掉前导 `../`)。**只用于取 media 裸文件名**
/// (`rsplit('/')` 末段,与主部件位置无关);要定位部件请用 `links::resolve_part_path`。
pub fn normalize_target(target: &str) -> String {
    let mut t = target;
    while let Some(rest) = t.strip_prefix("../") {
        t = rest;
    }
    // 相对 slide 部件,逻辑根是 `ppt/`,所以补回前缀(除非已经是绝对的 `/...`)。
    if let Some(stripped) = t.strip_prefix('/') {
        stripped.to_string()
    } else {
        format!("ppt/{t}")
    }
}

/// 取一个(可能带命名空间前缀的)元素名的本地名,如 `p:sp` -> `sp`。
pub fn local_name(qname: &[u8]) -> &[u8] {
    match qname.iter().position(|&b| b == b':') {
        Some(i) => &qname[i + 1..],
        None => qname,
    }
}

/// 把一个属性的值解码成 `String`(容错:解码失败给空串)。单值截到
/// [`budget::MAX_ATTR_BYTES`],并按实际字节计入当前模型字节预算(见 [`budget`])。
pub fn attr_string(attr: &quick_xml::events::attributes::Attribute) -> String {
    let mut s = attr
        .unescape_value()
        .map(|c| c.into_owned())
        .unwrap_or_default();
    budget::fit_string(&mut s, budget::MAX_ATTR_BYTES);
    s
}

/// 取元素的某个属性值(按本地名匹配,忽略命名空间前缀)。
pub fn attr_of(e: &BytesStart, key: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if local_name(attr.key.as_ref()) == key {
            return Some(attr_string(&attr));
        }
    }
    None
}

/// 读取一个 OOXML 布尔属性。OOXML 里 `b="1"` / `b="true"` 为真;缺失为假。
pub fn bool_attr(e: &BytesStart, key: &[u8]) -> bool {
    attr_of(e, key).map(ooxml_bool).unwrap_or(false)
}

/// OOXML 布尔字面量(`"1"` / `"true"` / `"on"` 为真)。
pub fn ooxml_bool(v: String) -> bool {
    v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("on")
}

/// 读取当前已打开元素的纯文本内容,直到其结束标签。已消费该元素的起始标签。单个文本节点截到
/// [`budget::MAX_TEXT_BYTES`],并按实际字节计入当前模型字节预算。
pub fn read_text<R: std::io::BufRead>(reader: &mut Reader<R>) -> String {
    read_text_capped(reader, budget::MAX_TEXT_BYTES)
}

/// 同 [`read_text`],单值上限为 `cap` 字节(图表标签等短值用更小的上限)。
pub fn read_text_capped<R: std::io::BufRead>(reader: &mut Reader<R>, cap: usize) -> String {
    let mut out = String::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Text(t)) => {
                if let Ok(s) = t.unescape() {
                    out.push_str(&s);
                }
            }
            Ok(Event::CData(c)) => {
                out.push_str(&String::from_utf8_lossy(&c));
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
        // 流式封顶:超长文本节点不先整段读进内存(截断判定留给下面的 fit_string)。
        if out.len() > cap {
            skip_element(reader, b"");
            break;
        }
    }
    budget::fit_string(&mut out, cap);
    out
}

/// 跳过当前已打开元素的全部内容,直到其匹配的结束标签。已消费该元素的起始标签。
/// 通过深度计数处理同名嵌套。
pub fn skip_element<R: std::io::BufRead>(reader: &mut Reader<R>, _name: &[u8]) {
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(_)) => depth += 1,
            Ok(Event::End(_)) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
}
