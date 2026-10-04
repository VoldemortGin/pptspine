//! 解析 `ppt/presentation.xml`:幻灯片画布尺寸 + 幻灯片呈现顺序(`r:id` 列表)
//! + 缺省文本样式(`p:defaultTextStyle`,非占位符文本框的继承基底)
//! + 节(扩展 `p14:sectionLst`,经 `sldId@id` 引用幻灯片)。

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
    /// 按 `p:sldIdLst` > `p:sldId` 顺序排列的 `r:id` 引用列表。
    pub slide_rids: Vec<String>,
    /// `p:defaultTextStyle`(层级列表样式;缺失为 `None`)。
    pub default_text_style: Option<TextStyleLevels>,
    /// `p:sldId` 的 `(@id, @r:id)` 对,按 `p:sldIdLst` 顺序(节经 `@id` 引用幻灯片)。
    pub slide_ids: Vec<(u32, String)>,
    /// 节(`p14:section`):`(name, [sldId@id])`,按文档顺序。
    pub sections: Vec<(String, Vec<u32>)>,
    /// `p:presentation@firstSlideNum`(缺失 / 非法为 `None`,调用方按缺省 1)。
    pub first_slide_num: Option<i32>,
}

/// 解析 `presentation.xml`。容错:遇错即返回已得部分。
pub fn parse(xml: &str) -> PresentationMeta {
    let mut meta = PresentationMeta::default();
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
                meta.sections.push((section_name(&e), Vec::new()));
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
                        meta.sections.push((section_name(&e), Vec::new()));
                        in_section = true;
                    }
                    b"sldId" if in_section => {
                        let id = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.as_ref() == b"id")
                            .and_then(|a| attr_string(&a).parse().ok());
                        if let (Some(id), Some(sect)) = (id, meta.sections.last_mut()) {
                            sect.1.push(id);
                        }
                    }
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
                        if let Some(rid) = rid {
                            if let Some(id) = num_id {
                                meta.slide_ids.push((id, rid.clone()));
                            }
                            meta.slide_rids.push(rid);
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
