//! quick-xml walker —— 按职责拆分:
//! - [`presentation`]:解析 `presentation.xml`(画布尺寸 + 幻灯片顺序)。
//! - [`slide`]:解析单张幻灯片 -> `Vec<Shape>`。
//!
//! 本模块根放**关系(`.rels`)解析**这类被多处复用的小工具。所有 walker 都遵循家族约定:
//! 未知元素跳过、缺失属性 → `None`、**绝不 panic**。

pub mod notes;
pub mod presentation;
pub mod slide;

use std::collections::BTreeMap;

use quick_xml::events::Event;
use quick_xml::Reader;

/// 一个 OOXML 关系条目(`<Relationship Id="rIdN" Type="..." Target="..."/>`)。
#[derive(Debug, Clone)]
pub struct Relationship {
    #[allow(dead_code)] // 关系 Id(rIdN)——保留为完整 API,暂未被内部消费
    pub id: String,
    pub rel_type: String,
    pub target: String,
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
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"Id" => id = attr_string(&attr),
                            b"Type" => rel_type = attr_string(&attr),
                            b"Target" => target = attr_string(&attr),
                            _ => {}
                        }
                    }
                    if !id.is_empty() {
                        map.insert(
                            id.clone(),
                            Relationship {
                                id,
                                rel_type,
                                target,
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

/// 在一份 `.rels` 里找到第一个 `Type` 包含 `kind` 子串的关系,返回其规范化 `Target`。
/// 例如 `kind = "slideLayout"`、`kind = "slideMaster"`。
pub fn first_rel_target_with(rels_xml: &str, kind: &str) -> Option<String> {
    let rels = parse_rels(rels_xml);
    rels.values()
        .find(|r| r.rel_type.contains(kind))
        .map(|r| normalize_target(&r.target))
}

/// 把关系 `Target` 规范化为相对 `ppt/` 根的部件路径(去掉前导 `../`)。
/// OOXML 里 slide 的 rels Target 形如 `../media/image1.png` 或 `../slideLayouts/slideLayout1.xml`。
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

/// 把一个属性的值解码成 `String`(容错:解码失败给空串)。
pub fn attr_string(attr: &quick_xml::events::attributes::Attribute) -> String {
    attr.unescape_value()
        .map(|c| c.into_owned())
        .unwrap_or_default()
}
