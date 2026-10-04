//! 解析 `ppt/presentation.xml`:幻灯片画布尺寸 + 幻灯片呈现顺序(`r:id` 列表)
//! + 缺省文本样式(`p:defaultTextStyle`,非占位符文本框的继承基底)
//! + 节(扩展 `p14:sectionLst`,经 `sldId@id` 引用幻灯片)。
//!
//! `p:sldIdLst` 边读边去重:每个 `sldId` 立即经调用方给的解析函数折成 slide 部件路径,重复引用
//! 只计数不存储,收集到 `max_slides + 1` 个不同部件就停止收集(多出的一个只为让调用方报超限)——
//! 一个压缩后几百 KB、展开上百 MB 的 `sldIdLst` 不会先被整个读进内存。

use std::collections::{BTreeMap, BTreeSet};

use ppt_core::style::TextStyleLevels;
use ppt_core::Emu;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::text_style::parse_list_style;
use super::{attr_of, attr_string, local_name};

/// `presentation.xml` 的解析结果。
#[derive(Debug, Clone, Default)]
pub struct PresentationMeta {
    /// 画布尺寸 `(cx, cy)`(EMU,来自 `p:sldSz`)。缺失时为 `(0, 0)`。
    pub slide_size: (Emu, Emu),
    /// 按 `p:sldIdLst` > `p:sldId` 顺序、去重后的 slide 部件路径(至多 `max_slides + 1` 个)。
    pub slide_parts: Vec<String>,
    /// 被重复引用的 slide 部件 → 被去掉的重复引用数。
    pub duplicate_refs: BTreeMap<String, usize>,
    /// 不同 slide 部件数超过 `max_slides`(之后的引用未收集)。
    pub too_many_slides: bool,
    /// `p:defaultTextStyle`(层级列表样式;缺失为 `None`)。
    pub default_text_style: Option<TextStyleLevels>,
    /// 被收集的 `p:sldId` 的 `(@id, slide 部件路径)` 对,按 `p:sldIdLst` 顺序(节经 `@id` 引用幻灯片)。
    pub slide_ids: Vec<(u32, String)>,
    /// 节(`p14:section`):`(name, [sldId@id])`,按文档顺序。
    pub sections: Vec<(String, Vec<u32>)>,
    /// `p:presentation@firstSlideNum`(缺失 / 非法为 `None`,调用方按缺省 1)。
    pub first_slide_num: Option<i32>,
}

/// 解析 `presentation.xml`。容错:遇错即返回已得部分。`resolve` 把 `sldId@r:id` 折成存在的
/// slide 部件路径(折不出的引用丢弃);不同部件至多收集 `max_slides + 1` 个。节里的 `sldId`
/// 总数同样至多 `max_slides`(一张幻灯片只属于一个节)。
pub fn parse(
    xml: &str,
    max_slides: usize,
    mut resolve: impl FnMut(&str) -> Option<String>,
) -> PresentationMeta {
    let mut meta = PresentationMeta::default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut section_ids = 0usize;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    // 当前所在的 `p14:section`(其 `sldId` 只带 `@id`,与主 `sldIdLst` 区分开)。
    let mut in_section = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::End(e)) if local_name(e.name().as_ref()) == b"section" => in_section = false,
            // 自闭合的空节:登记但不进入"节内"状态。
            Ok(Event::Empty(e)) if local_name(e.name().as_ref()) == b"section" => {
                if meta.sections.len() < max_slides {
                    meta.sections.push((section_name(&e), Vec::new()));
                }
            }
            Ok(Event::Start(e)) if local_name(e.name().as_ref()) == b"defaultTextStyle" => {
                let ls = parse_list_style(&mut reader);
                if !ls.is_empty() {
                    meta.default_text_style = Some(ls);
                }
            }
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"presentation" => {
                        meta.first_slide_num =
                            attr_of(&e, b"firstSlideNum").and_then(|v| v.trim().parse().ok());
                    }
                    b"sldSz" => {
                        let mut cx: Emu = 0;
                        let mut cy: Emu = 0;
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"cx" => cx = attr_string(&attr).parse().unwrap_or(0),
                                b"cy" => cy = attr_string(&attr).parse().unwrap_or(0),
                                _ => {}
                            }
                        }
                        meta.slide_size = (cx, cy);
                    }
                    b"section" => {
                        if meta.sections.len() < max_slides {
                            meta.sections.push((section_name(&e), Vec::new()));
                            in_section = true;
                        }
                    }
                    b"sldId" if in_section => {
                        let id = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.as_ref() == b"id")
                            .and_then(|a| attr_string(&a).parse().ok());
                        if let (Some(id), Some(sect)) = (id, meta.sections.last_mut()) {
                            if section_ids < max_slides {
                                section_ids += 1;
                                sect.1.push(id);
                            }
                        }
                    }
                    b"sldId" if meta.too_many_slides => {}
                    b"sldId" => {
                        // `r:id` 属性引用 presentation 的 rels 里的一条关系;`@id` 供节引用。
                        let mut num_id: Option<u32> = None;
                        let mut rid: Option<String> = None;
                        for attr in e.attributes().flatten() {
                            if local_name(attr.key.as_ref()) == b"id"
                                && attr.key.as_ref().starts_with(b"r:")
                            {
                                rid = Some(attr_string(&attr));
                            } else if attr.key.as_ref() == b"id" {
                                num_id = attr_string(&attr).parse().ok();
                            }
                        }
                        let Some(target) = rid.as_deref().and_then(&mut resolve) else {
                            buf.clear();
                            continue;
                        };
                        if seen.contains(&target) {
                            *meta.duplicate_refs.entry(target).or_default() += 1;
                        } else if seen.len() > max_slides {
                            meta.too_many_slides = true;
                        } else {
                            seen.insert(target.clone());
                            if let Some(id) = num_id {
                                meta.slide_ids.push((id, target.clone()));
                            }
                            meta.slide_parts.push(target);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    meta
}

/// `p14:section@name`(缺失为空串)。
fn section_name(e: &BytesStart) -> String {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == b"name")
        .map(|a| attr_string(&a))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 5 万个重复的 `sldId` + 1 000 个不同引用、上限 10:重复只计数不存储,不同部件收集到 11 个
    /// 就停(之后不再折算路径),节里的 `sldId` 同样封顶。
    #[test]
    fn sld_id_list_is_deduplicated_while_reading_and_stops_after_max_plus_one() {
        let dup = r#"<p:sldId id="256" r:id="rId1"/>"#.repeat(50_000);
        let distinct: String = (0..1_000)
            .map(|i| format!(r#"<p:sldId id="{}" r:id="r{i}"/>"#, 300 + i))
            .collect();
        let sect = format!(
            r#"<p14:sectionLst xmlns:p14="p14"><p14:section name="s"><p14:sldIdLst>{}</p14:sldIdLst></p14:section></p14:sectionLst>"#,
            r#"<p14:sldId id="256"/>"#.repeat(5_000)
        );
        let xml = format!(
            r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldIdLst>{dup}{distinct}</p:sldIdLst>{sect}</p:presentation>"#
        );
        let mut resolved = 0usize;
        let meta = parse(&xml, 10, |rid| {
            resolved += 1;
            Some(format!("ppt/slides/{rid}.xml"))
        });
        assert_eq!(meta.slide_parts.len(), 11);
        assert_eq!(meta.slide_ids.len(), 11);
        assert!(meta.too_many_slides);
        assert_eq!(
            meta.duplicate_refs.get("ppt/slides/rId1.xml"),
            Some(&49_999)
        );
        assert_eq!(resolved, 50_000 + 11, "超限后不再折算");
        assert_eq!(meta.sections[0].1.len(), 10);
    }
}
