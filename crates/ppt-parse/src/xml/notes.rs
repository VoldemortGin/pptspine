//! 解析演讲者备注 `ppt/notesSlides/notesSlideN.xml` -> 备注文本。
//!
//! 走 `p:notes` > `p:cSld` > `p:spTree`,在其直接子 `p:sp` 里找演讲者备注:优先取
//! `<p:ph type="body">` 占位符的文字;若没有 body 占位符,则回退为全部文本框文字
//! (容错兜底)。容错:未知元素跳过、无文字返回 `None`、绝不 panic。

use quick_xml::events::Event;
use quick_xml::Reader;

use super::local_name;

/// 从一份 notesSlide XML 提取演讲者备注文本。无文字返回 `None`。
pub fn parse(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    // 先定位到 spTree,再收集其直接子形状里的备注文字。
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == b"spTree" {
                    return collect_notes(&mut reader);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    None
}

/// 收集 spTree 里各 `p:sp` 的文字:优先 body 占位符,兜底取全部。已消费 `<p:spTree>` 起始标签。
fn collect_notes<R: std::io::BufRead>(reader: &mut Reader<R>) -> Option<String> {
    let mut body: Vec<String> = Vec::new();
    let mut all: Vec<String> = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"sp" {
                    let (is_body, paras) = parse_notes_sp(reader);
                    let paras: Vec<String> =
                        paras.into_iter().filter(|p| !p.trim().is_empty()).collect();
                    if !paras.is_empty() {
                        if is_body {
                            body.extend(paras.iter().cloned());
                        }
                        all.extend(paras);
                    }
                } else {
                    skip_element(reader);
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    let chosen = if !body.is_empty() { body } else { all };
    let joined = chosen.join("\n");
    if joined.trim().is_empty() {
        None
    } else {
        Some(joined)
    }
}

/// 解析一个 `p:sp`,返回 `(是否 body 占位符, 各段文字)`。已消费 `<p:sp>` 起始标签。
fn parse_notes_sp<R: std::io::BufRead>(reader: &mut Reader<R>) -> (bool, Vec<String>) {
    let mut is_body = false;
    let mut paras: Vec<String> = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"nvSpPr" => is_body = find_body_ph(reader) || is_body,
                    b"txBody" => paras = txbody_paragraphs(reader),
                    _ => skip_element(reader),
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    (is_body, paras)
}

/// 在 `p:nvSpPr` 子树里找 `<p:ph type="body">`。已消费 `<p:nvSpPr>` 起始标签。
fn find_body_ph<R: std::io::BufRead>(reader: &mut Reader<R>) -> bool {
    let mut found = false;
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                if local_name(e.name().as_ref()) == b"ph" && ph_is_body(&e) {
                    found = true;
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"ph" && ph_is_body(&e) {
                    found = true;
                }
            }
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
    found
}

/// `<p:ph type="body">`?(备注占位符的类型为 `body`)。
fn ph_is_body(e: &quick_xml::events::BytesStart) -> bool {
    for attr in e.attributes().flatten() {
        if local_name(attr.key.as_ref()) == b"type" {
            return super::attr_string(&attr) == "body";
        }
    }
    false
}

/// 解析 `p:txBody` -> 各段文字。已消费 `<p:txBody>` 起始标签。
fn txbody_paragraphs<R: std::io::BufRead>(reader: &mut Reader<R>) -> Vec<String> {
    let mut paras = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"p" {
                    paras.push(paragraph_text(reader));
                } else {
                    skip_element(reader);
                }
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    paras
}

/// 取 `a:p` 段落里所有 `a:t`(含 `a:fld` 等)内的纯文本。已消费 `<a:p>` 起始标签。
fn paragraph_text<R: std::io::BufRead>(reader: &mut Reader<R>) -> String {
    let mut out = String::new();
    let mut depth = 1usize; // a:p 已打开
    let mut in_t = 0usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                if local_name(e.name().as_ref()) == b"t" {
                    in_t += 1;
                }
            }
            Ok(Event::Text(t)) => {
                if in_t > 0 {
                    if let Ok(s) = t.unescape() {
                        out.push_str(&s);
                    }
                }
            }
            Ok(Event::CData(c)) => {
                if in_t > 0 {
                    out.push_str(&String::from_utf8_lossy(&c));
                }
            }
            Ok(Event::End(e)) => {
                if local_name(e.name().as_ref()) == b"t" && in_t > 0 {
                    in_t -= 1;
                }
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
    out
}

/// 跳过当前已打开元素的全部内容,直到其匹配的结束标签。已消费该元素的起始标签。
fn skip_element<R: std::io::BufRead>(reader: &mut Reader<R>) {
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

#[cfg(test)]
mod tests {
    use super::*;

    const NOTES_XML: &str = r#"<?xml version="1.0"?>
    <p:notes xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
             xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
      <p:cSld><p:spTree>
        <p:sp>
          <p:nvSpPr><p:nvPr><p:ph type="sldImg"/></p:nvPr></p:nvSpPr>
          <p:txBody><a:p><a:r><a:t>should not be picked</a:t></a:r></a:p></p:txBody>
        </p:sp>
        <p:sp>
          <p:nvSpPr><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
          <p:txBody>
            <a:p><a:r><a:t>Line one</a:t></a:r></a:p>
            <a:p><a:r><a:t>Line two</a:t></a:r></a:p>
          </p:txBody>
        </p:sp>
      </p:spTree></p:cSld>
    </p:notes>"#;

    #[test]
    fn picks_body_placeholder() {
        let got = parse(NOTES_XML).unwrap();
        assert_eq!(got, "Line one\nLine two");
    }

    #[test]
    fn empty_notes_is_none() {
        let xml = r#"<p:notes xmlns:p="x"><p:cSld><p:spTree></p:spTree></p:cSld></p:notes>"#;
        assert!(parse(xml).is_none());
    }

    #[test]
    fn fallback_when_no_body() {
        let xml = r#"<p:notes xmlns:a="a" xmlns:p="p"><p:cSld><p:spTree>
          <p:sp><p:txBody><a:p><a:r><a:t>only text</a:t></a:r></a:p></p:txBody></p:sp>
        </p:spTree></p:cSld></p:notes>"#;
        assert_eq!(parse(xml).as_deref(), Some("only text"));
    }
}
