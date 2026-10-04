//! 解析 SmartArt data 部件 `ppt/diagrams/dataN.xml`(`dgm:dataModel`):
//! 抽取内容点(`dgm:ptLst > dgm:pt`)的文字,并读出 drawing 部件的关系 id。
//!
//! `dgm:pt > dgm:t` 与 `p:txBody` 同构,复用 [`super::slide::parse_txbody`]。`type` 为
//! `pres`(呈现点)/ `parTrans` / `sibTrans`(连线过渡点)的是布局辅助点,不是内容,跳过。
//! 容错:未知元素跳过、畸形输入返回已得部分、绝不 panic。

use quick_xml::events::Event;
use quick_xml::Reader;

use super::slide::parse_txbody;
use super::{attr_of, local_name, skip_element};

/// data 部件的解析结果。
#[derive(Debug, Clone, Default)]
pub struct DiagramData {
    /// 内容点的非空段落文字,文档顺序。
    pub texts: Vec<String>,
    /// `dgm:extLst > a:ext > dsp:dataModelExt@relId`:drawing 部件在**幻灯片** rels 里的 id。
    pub drawing_rel_id: Option<String>,
    /// 解析 `dgm:t` 文字时因嵌套超限被跳过的子树数(诊断用)。
    pub nesting_skipped: usize,
}

/// 解析一份 data 部件 XML。
pub fn parse_data(xml: &str) -> DiagramData {
    let mut out = DiagramData::default();
    super::slide::reset_nest_skipped();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"pt" => {
                        let ty = attr_of(&e, b"type");
                        if matches!(ty.as_deref(), Some("pres" | "parTrans" | "sibTrans")) {
                            skip_element(&mut reader, &name);
                        } else {
                            parse_pt(&mut reader, &mut out.texts);
                        }
                    }
                    b"dataModelExt" => {
                        out.drawing_rel_id = attr_of(&e, b"relId").or(out.drawing_rel_id.take());
                        skip_element(&mut reader, &name);
                    }
                    // dataModel / ptLst / extLst / ext 等容器继续下钻;其余整体跳过。
                    b"dataModel" | b"ptLst" | b"extLst" | b"ext" => {}
                    _ => skip_element(&mut reader, &name),
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"dataModelExt" {
                    out.drawing_rel_id = attr_of(&e, b"relId").or(out.drawing_rel_id.take());
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out.nesting_skipped = super::slide::take_nest_skipped();
    out
}

/// 一个 `dgm:pt` 内:取 `dgm:t` 的段落文字(空段落丢弃)。已消费 `<dgm:pt>` 起始标签。
fn parse_pt<R: std::io::BufRead>(reader: &mut Reader<R>, texts: &mut Vec<String>) {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"t" {
                    for p in parse_txbody(reader).paragraphs {
                        let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
                        if !text.trim().is_empty() {
                            texts.push(text);
                        }
                    }
                } else {
                    skip_element(reader, &name);
                }
            }
            Ok(Event::End(_) | Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
}
