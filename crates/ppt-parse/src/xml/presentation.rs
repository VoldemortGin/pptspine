//! 解析 `ppt/presentation.xml`:幻灯片画布尺寸 + 幻灯片呈现顺序(`r:id` 列表)
//! + 缺省文本样式(`p:defaultTextStyle`,非占位符文本框的继承基底)。

use ppt_core::style::TextStyleLevels;
use ppt_core::Emu;
use quick_xml::events::Event;
use quick_xml::Reader;

use super::text_style::parse_list_style;
use super::{attr_string, local_name};

/// `presentation.xml` 的解析结果。
#[derive(Debug, Clone, Default)]
pub struct PresentationMeta {
    /// 画布尺寸 `(cx, cy)`(EMU,来自 `p:sldSz`)。缺失时为 `(0, 0)`。
    pub slide_size: (Emu, Emu),
    /// 按 `p:sldIdLst` > `p:sldId` 顺序排列的 `r:id` 引用列表。
    pub slide_rids: Vec<String>,
    /// `p:defaultTextStyle`(层级列表样式;缺失为 `None`)。
    pub default_text_style: Option<TextStyleLevels>,
}

/// 解析 `presentation.xml`。容错:遇错即返回已得部分。
pub fn parse(xml: &str) -> PresentationMeta {
    let mut meta = PresentationMeta::default();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if local_name(e.name().as_ref()) == b"defaultTextStyle" => {
                let ls = parse_list_style(&mut reader);
                if !ls.is_empty() {
                    meta.default_text_style = Some(ls);
                }
            }
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
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
                    b"sldId" => {
                        // `r:id` 属性引用 presentation 的 rels 里的一条关系。
                        for attr in e.attributes().flatten() {
                            if local_name(attr.key.as_ref()) == b"id"
                                && attr.key.as_ref().starts_with(b"r:")
                            {
                                meta.slide_rids.push(attr_string(&attr));
                            }
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
