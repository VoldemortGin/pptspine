//! 解析文档属性部件:`docProps/core.xml`(OPC core properties,Dublin Core)与
//! `docProps/app.xml`(扩展属性)-> [`DocProperties`]。
//!
//! 只认一层扁平子元素(按本地名匹配,忽略命名空间前缀),值原样取文本(去首尾空白,
//! 空串按缺失)。容错:畸形 XML 返回已得部分。

use ppt_core::model::DocProperties;
use quick_xml::events::Event;
use quick_xml::Reader;

use super::{local_name, skip_element};

/// 把 `core.xml` 的字段填进 `props`(已有值被覆盖)。
pub fn parse_core(xml: &str, props: &mut DocProperties) {
    walk(xml, |name, value| {
        let slot = match name {
            b"title" => &mut props.title,
            b"subject" => &mut props.subject,
            b"creator" => &mut props.creator,
            b"keywords" => &mut props.keywords,
            b"description" => &mut props.description,
            b"category" => &mut props.category,
            b"lastModifiedBy" => &mut props.last_modified_by,
            b"revision" => &mut props.revision,
            b"created" => &mut props.created,
            b"modified" => &mut props.modified,
            b"language" => &mut props.language,
            _ => return,
        };
        *slot = Some(value);
    });
}

/// 把 `app.xml` 的字段填进 `props`(已有值被覆盖)。
pub fn parse_app(xml: &str, props: &mut DocProperties) {
    walk(xml, |name, value| {
        let slot = match name {
            b"Application" => &mut props.application,
            b"AppVersion" => &mut props.app_version,
            b"Company" => &mut props.company,
            b"Manager" => &mut props.manager,
            b"PresentationFormat" => &mut props.presentation_format,
            _ => return,
        };
        *slot = Some(value);
    });
}

/// 逐个根下直接子元素回调 `(本地名, 文本)`;嵌套容器(如 app.xml 的
/// `HeadingPairs`)整体按文本读掉、不回调已知字段以外的内容。
fn walk(xml: &str, mut f: impl FnMut(&[u8], String)) {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                if depth == 2 {
                    let name = local_name(e.name().as_ref()).to_vec();
                    let text = read_text_deep(&mut reader);
                    depth -= 1;
                    let text = text.trim();
                    if !text.is_empty() {
                        f(&name, text.to_string());
                    }
                }
            }
            Ok(Event::End(_)) => depth = depth.saturating_sub(1),
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
}

/// 读当前元素(已消费起始标签)直到匹配结束标签;只收集**直接**文本,子元素整体跳过
/// (`super::read_text` 遇第一个结束标签即停,不适合含子元素的容器)。
fn read_text_deep<R: std::io::BufRead>(reader: &mut Reader<R>) -> String {
    let mut out = String::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Text(t)) => {
                if let Ok(s) = t.unescape() {
                    out.push_str(&s);
                }
            }
            // 子元素:其文本不并入,整棵跳过(`skip_element` 处理同名嵌套)。
            Ok(Event::Start(e)) => skip_element(reader, e.name().as_ref()),
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_and_app_fields() {
        let core = r#"<?xml version="1.0"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
  xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/"
  xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <dc:title>Q3 Review</dc:title><dc:creator>Ada &amp; Co</dc:creator>
  <cp:lastModifiedBy>Bob</cp:lastModifiedBy><cp:revision>7</cp:revision>
  <dcterms:created xsi:type="dcterms:W3CDTF">2026-01-02T03:04:05Z</dcterms:created>
  <dcterms:modified xsi:type="dcterms:W3CDTF">2026-02-03T04:05:06Z</dcterms:modified>
  <dc:subject></dc:subject>
</cp:coreProperties>"#;
        let app = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"
  xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">
  <Application>Microsoft Office PowerPoint</Application>
  <HeadingPairs><vt:vector size="2" baseType="variant"><vt:variant><vt:lpstr>Theme</vt:lpstr></vt:variant></vt:vector></HeadingPairs>
  <Company>Acme</Company><AppVersion>16.0000</AppVersion>
</Properties>"#;
        let mut p = DocProperties::default();
        parse_core(core, &mut p);
        parse_app(app, &mut p);
        assert_eq!(p.title.as_deref(), Some("Q3 Review"));
        assert_eq!(p.creator.as_deref(), Some("Ada & Co"));
        assert_eq!(p.last_modified_by.as_deref(), Some("Bob"));
        assert_eq!(p.revision.as_deref(), Some("7"));
        assert_eq!(p.created.as_deref(), Some("2026-01-02T03:04:05Z"));
        assert_eq!(p.modified.as_deref(), Some("2026-02-03T04:05:06Z"));
        assert_eq!(p.subject, None);
        assert_eq!(
            p.application.as_deref(),
            Some("Microsoft Office PowerPoint")
        );
        assert_eq!(p.company.as_deref(), Some("Acme"));
        assert_eq!(p.app_version.as_deref(), Some("16.0000"));
        assert_eq!(p.manager, None);
    }

    #[test]
    fn malformed_is_tolerated() {
        let mut p = DocProperties::default();
        parse_core("<cp:coreProperties><dc:title>T</dc:title><broken", &mut p);
        assert_eq!(p.title.as_deref(), Some("T"));
    }
}
