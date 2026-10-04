//! zip 读取限额([`ZipLimits`])与形状嵌套深度守卫的验收测试。
//!
//! 全部在测试里用 `zip` 现场合成(含按字节改写头字段伪造声明大小),不落二进制 fixture。
//! 每个恶意输入都必须返回类型化错误,不 panic、不按声明值巨量预分配。

use std::io::{Cursor, Write};

use ppt_core::model::Shape;
use ppt_core::PptError;
use ppt_parse::{parse_bytes, parse_bytes_with_limits, LimitKind, ZipLimits};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const PRESENTATION: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
                xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst>
  <p:sldSz cx="9144000" cy="6858000"/>
</p:presentation>"#;

const PRESENTATION_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"#;

fn slide_with_sp_tree(inner: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>{inner}</p:spTree></p:cSld>
</p:sld>"#
    )
}

const SIMPLE_SP: &str = r#"<p:sp><p:txBody><a:p><a:r><a:t>hi</a:t></a:r></a:p></p:txBody></p:sp>"#;

/// 最小合法 pptx + 额外条目(`(名字, 字节, 压缩方式)`)。
fn build(slide_xml: &str, extra: &[(&str, &[u8], CompressionMethod)]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default();
        for (name, body) in [
            ("ppt/presentation.xml", PRESENTATION.as_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                PRESENTATION_RELS.as_bytes(),
            ),
            ("ppt/slides/slide1.xml", slide_xml.as_bytes()),
        ] {
            zip.start_file(name, opts).expect("start_file");
            zip.write_all(body).expect("write");
        }
        for (name, body, method) in extra {
            zip.start_file(*name, opts.compression_method(*method))
                .expect("start_file extra");
            zip.write_all(body).expect("write extra");
        }
        zip.finish().expect("finish zip");
    }
    buf.into_inner()
}

fn simple_with(extra: &[(&str, &[u8], CompressionMethod)]) -> Vec<u8> {
    build(&slide_with_sp_tree(SIMPLE_SP), extra)
}

/// 确定性的"不可压缩"字节(LCG),压缩比约 1。
fn noise(len: usize) -> Vec<u8> {
    let mut x: u32 = 0x1234_5678;
    (0..len)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect()
}

/// 把条目 `name` 在本地头与中央目录里的"未压缩大小"字段改写成 `size`(伪造声明值)。
fn patch_declared_size(bytes: &mut [u8], name: &str, size: u32) {
    let name = name.as_bytes();
    let le = size.to_le_bytes();
    let mut patched = 0;
    let mut i = 0;
    while i + 4 <= bytes.len() {
        // (名字偏移, 名字长度字段偏移, 未压缩大小字段偏移)
        let (name_at, name_len_at, size_at) = match &bytes[i..i + 4] {
            b"PK\x03\x04" => (30, 26, 22),
            b"PK\x01\x02" => (46, 28, 24),
            _ => {
                i += 1;
                continue;
            }
        };
        let n = u16::from_le_bytes([bytes[i + name_len_at], bytes[i + name_len_at + 1]]) as usize;
        if bytes.get(i + name_at..i + name_at + n) == Some(name) {
            bytes[i + size_at..i + size_at + 4].copy_from_slice(&le);
            patched += 1;
        }
        i += 4;
    }
    assert_eq!(patched, 2, "expected local + central header for entry");
}

fn expect_limit(res: ppt_core::Result<ppt_parse::ParsedPptx>, want: LimitKind) -> (u64, u64) {
    match res {
        Err(PptError::LimitExceeded {
            kind,
            limit,
            actual,
        }) => {
            assert_eq!(kind, want);
            assert!(
                actual > limit,
                "actual {actual} should exceed limit {limit}"
            );
            (limit, actual)
        }
        Err(other) => panic!("expected LimitExceeded({want:?}), got error {other}"),
        Ok(_) => panic!("expected LimitExceeded({want:?}), got Ok"),
    }
}

#[test]
fn default_limits_values() {
    let d = ZipLimits::default();
    assert_eq!(d.max_entries, 10_000);
    assert_eq!(d.max_entry_bytes, 256 * 1024 * 1024);
    assert_eq!(d.max_total_bytes, 1024 * 1024 * 1024);
    assert_eq!(d.max_compression_ratio, 10_000);
    assert_eq!(d.max_name_len, 1024);
    assert_eq!(d.max_slides, 5_000);
}

#[test]
fn normal_document_unchanged_under_default_limits() {
    let bytes = simple_with(&[(
        "ppt/media/image1.png",
        &noise(4096),
        CompressionMethod::Deflated,
    )]);
    let a = parse_bytes(&bytes).expect("default parse");
    let b = parse_bytes_with_limits(&bytes, &ZipLimits::default()).expect("explicit default");
    assert_eq!(a.presentation.slides.len(), 1);
    assert_eq!(a.presentation.slides.len(), b.presentation.slides.len());
    assert_eq!(a.media.get("image1.png").map(Vec::len), Some(4096));
    assert_eq!(a.media, b.media);
}

#[test]
fn too_many_entries() {
    let names: Vec<String> = (0..10_001).map(|i| format!("ppt/x/{i}.xml")).collect();
    let extra: Vec<(&str, &[u8], CompressionMethod)> = names
        .iter()
        .map(|n| (n.as_str(), &b""[..], CompressionMethod::Stored))
        .collect();
    let (limit, actual) = expect_limit(parse_bytes(&simple_with(&extra)), LimitKind::Entries);
    assert_eq!(limit, 10_000);
    assert_eq!(actual, 10_004);
}

#[test]
fn name_too_long() {
    let name = format!("ppt/{}", "a".repeat(1100));
    let bytes = simple_with(&[(&name, b"x", CompressionMethod::Stored)]);
    let (limit, actual) = expect_limit(parse_bytes(&bytes), LimitKind::NameLength);
    assert_eq!(limit, 1024);
    assert_eq!(actual, name.len() as u64);
}

#[test]
fn parent_dir_and_absolute_paths_rejected() {
    for bad in [
        "../evil.xml",
        "ppt/../../evil.xml",
        "ppt/a/../b.xml",
        "/abs.xml",
        "C:\\x",
        "C:/x",
    ] {
        let bytes = simple_with(&[(bad, b"x", CompressionMethod::Stored)]);
        match parse_bytes(&bytes) {
            Err(PptError::Zip(msg)) => assert!(msg.contains("unsafe entry path"), "{bad}: {msg}"),
            Err(other) => panic!("{bad}: unexpected error {other}"),
            Ok(_) => panic!("{bad}: expected rejection"),
        }
    }
}

#[test]
fn declared_size_over_entry_limit() {
    let limits = ZipLimits {
        max_entry_bytes: 1000,
        ..ZipLimits::default()
    };
    let bytes = simple_with(&[("ppt/media/a.bin", &noise(2000), CompressionMethod::Stored)]);
    let (limit, actual) = expect_limit(
        parse_bytes_with_limits(&bytes, &limits),
        LimitKind::EntryBytes,
    );
    assert_eq!((limit, actual), (1000, 2000));
}

#[test]
fn forged_huge_declared_size_rejected_without_allocation() {
    // 声明 ~4 GiB(真实只有 16 字节):旧实现会按声明值 with_capacity 预分配。
    let mut bytes = simple_with(&[("ppt/media/a.bin", &noise(16), CompressionMethod::Deflated)]);
    patch_declared_size(&mut bytes, "ppt/media/a.bin", 0xFFFF_FFFE);
    let (limit, actual) = expect_limit(parse_bytes(&bytes), LimitKind::EntryBytes);
    assert_eq!(limit, 256 * 1024 * 1024);
    assert_eq!(actual, 0xFFFF_FFFE);
}

#[test]
fn forged_small_declared_size_caught_while_reading() {
    // 声明 100 字节,实际解压 2 MiB:靠 take(limit + 1) 截断读取兜底。
    let limits = ZipLimits {
        max_entry_bytes: 1024 * 1024,
        ..ZipLimits::default()
    };
    let mut bytes = simple_with(&[(
        "ppt/media/a.bin",
        &noise(2 * 1024 * 1024),
        CompressionMethod::Deflated,
    )]);
    patch_declared_size(&mut bytes, "ppt/media/a.bin", 100);
    let (limit, actual) = expect_limit(
        parse_bytes_with_limits(&bytes, &limits),
        LimitKind::EntryBytes,
    );
    assert_eq!(limit, 1024 * 1024);
    assert_eq!(actual, 1024 * 1024 + 1);
}

#[test]
fn compression_ratio_bomb_rejected() {
    let limits = ZipLimits {
        max_compression_ratio: 100,
        ..ZipLimits::default()
    };
    let zeros = vec![0u8; 4 * 1024 * 1024];
    let bytes = simple_with(&[("ppt/media/zeros.bin", &zeros, CompressionMethod::Deflated)]);
    let (limit, _) = expect_limit(
        parse_bytes_with_limits(&bytes, &limits),
        LimitKind::CompressionRatio,
    );
    assert_eq!(limit, 100);
}

#[test]
fn real_4mib_zeros_pass_under_default_limits() {
    // deflate 对零的比值约 1032:1,旧默认 1000 会误拒;默认 10 000 下应通过。
    let zeros = vec![0u8; 4 * 1024 * 1024];
    let bytes = simple_with(&[("ppt/media/zeros.bin", &zeros, CompressionMethod::Deflated)]);
    let parsed = parse_bytes(&bytes).expect("legit solid-color media allowed");
    assert_eq!(
        parsed.media.get("zeros.bin").map(Vec::len),
        Some(4 * 1024 * 1024)
    );
}

#[test]
fn compression_ratio_not_checked_for_small_entries() {
    // 512 KiB 零(压缩比远超 100,但未过 1 MiB 起判门槛)→ 不误伤。
    let limits = ZipLimits {
        max_compression_ratio: 100,
        ..ZipLimits::default()
    };
    let zeros = vec![0u8; 512 * 1024];
    let bytes = simple_with(&[("ppt/media/zeros.bin", &zeros, CompressionMethod::Deflated)]);
    let parsed = parse_bytes_with_limits(&bytes, &limits).expect("small entry allowed");
    assert_eq!(
        parsed.media.get("zeros.bin").map(Vec::len),
        Some(512 * 1024)
    );
}

#[test]
fn total_bytes_over_limit() {
    let limits = ZipLimits {
        max_total_bytes: 1024 * 1024,
        ..ZipLimits::default()
    };
    let chunk = noise(400 * 1024);
    let bytes = simple_with(&[
        ("ppt/media/a.bin", &chunk, CompressionMethod::Stored),
        ("ppt/media/b.bin", &chunk, CompressionMethod::Stored),
        ("ppt/media/c.bin", &chunk, CompressionMethod::Stored),
    ]);
    let (limit, actual) = expect_limit(
        parse_bytes_with_limits(&bytes, &limits),
        LimitKind::TotalBytes,
    );
    assert_eq!(limit, 1024 * 1024);
    // 累计值:400K + 400K + 截断读取的 248K + 1 = max_total_bytes + 1。
    assert_eq!(actual, limit + 1);
}

#[test]
fn limit_error_message_names_the_limit() {
    let limits = ZipLimits {
        max_entry_bytes: 1000,
        ..ZipLimits::default()
    };
    let bytes = simple_with(&[("ppt/media/a.bin", &noise(2000), CompressionMethod::Stored)]);
    let err = parse_bytes_with_limits(&bytes, &limits).expect_err("limit error");
    assert_eq!(err.kind(), "limit-exceeded");
    assert_eq!(
        err.to_string(),
        "limit exceeded: entry-bytes (limit 1000, actual 2000)"
    );
}

fn nested_groups(depth: usize) -> String {
    let mut s = String::with_capacity(depth * 24 + SIMPLE_SP.len());
    for _ in 0..depth {
        s.push_str("<p:grpSp>");
    }
    s.push_str(SIMPLE_SP);
    for _ in 0..depth {
        s.push_str("</p:grpSp>");
    }
    s
}

fn group_chain_depth(shapes: &[Shape]) -> usize {
    let mut depth = 0;
    let mut cur = shapes;
    while let Some(Shape::Group(g)) = cur.first() {
        depth += 1;
        cur = &g.children;
    }
    depth
}

#[test]
fn shallow_group_nesting_keeps_innermost_shape() {
    let bytes = build(&slide_with_sp_tree(&nested_groups(10)), &[]);
    let parsed = parse_bytes(&bytes).expect("parse");
    let shapes = &parsed.presentation.slides[0].shapes;
    assert_eq!(group_chain_depth(shapes), 10);
}

#[test]
fn deep_group_nesting_is_truncated_not_overflowed() {
    // 远超守卫深度:旧实现递归下降会爆栈 abort;现在超过 64 层的子树整体跳过。
    let bytes = build(&slide_with_sp_tree(&nested_groups(100_000)), &[]);
    let parsed = parse_bytes(&bytes).expect("parse");
    let shapes = &parsed.presentation.slides[0].shapes;
    assert_eq!(group_chain_depth(shapes), 64);
}

// ---------------------------------------------------------------- 重复引用放大 / 幻灯片总数上限

/// 合成一个多 slide 的 pptx:`slides[i]` 是第 i 个部件 `slideN.xml` 的文字;`refs` 是
/// `p:sldIdLst` 里按顺序引用的部件序号(可重复),每个引用用独立的 `rIdK`。
fn deck(slide_texts: &[&str], refs: &[usize]) -> Vec<u8> {
    let ids: String = refs
        .iter()
        .enumerate()
        .map(|(k, _)| format!(r#"<p:sldId id="{}" r:id="rId{}"/>"#, 256 + k, k + 1))
        .collect();
    let pres = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
                xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/>
</p:presentation>"#
    );
    let rel_entries: String = refs
        .iter()
        .enumerate()
        .map(|(k, n)| {
            format!(
                r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{}.xml"/>"#,
                k + 1,
                n + 1
            )
        })
        .collect();
    let rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rel_entries}</Relationships>"#
    );
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default();
        let mut put = |name: &str, body: &str| {
            zip.start_file(name, opts).expect("start_file");
            zip.write_all(body.as_bytes()).expect("write");
        };
        put("ppt/presentation.xml", &pres);
        put("ppt/_rels/presentation.xml.rels", &rels);
        for (i, text) in slide_texts.iter().enumerate() {
            let sp = format!(
                r#"<p:sp><p:txBody><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"#
            );
            put(
                &format!("ppt/slides/slide{}.xml", i + 1),
                &slide_with_sp_tree(&sp),
            );
        }
        zip.finish().expect("finish zip");
    }
    buf.into_inner()
}

fn slide_text(parsed: &ppt_parse::ParsedPptx, i: usize) -> String {
    let Shape::TextBox(tf) = &parsed.presentation.slides[i].shapes[0] else {
        panic!("expected a text box");
    };
    tf.paragraphs[0].runs[0].text.clone()
}

/// 同一张幻灯片在 `p:sldIdLst` 里被引用 1000 次:只解析并保存一份。
#[test]
fn slide_referenced_1000_times_is_parsed_once() {
    let parsed = parse_bytes(&deck(&["only"], &[0; 1000])).expect("parse");
    assert_eq!(parsed.presentation.slides.len(), 1);
    assert_eq!(slide_text(&parsed, 0), "only");
}

/// 重复引用只保留首次出现,保持顺序:A B A C B -> A B C。
#[test]
fn duplicate_slide_refs_keep_first_occurrence_order() {
    let parsed = parse_bytes(&deck(&["A", "B", "C"], &[0, 1, 0, 2, 1])).expect("parse");
    let texts: Vec<_> = (0..parsed.presentation.slides.len())
        .map(|i| slide_text(&parsed, i))
        .collect();
    assert_eq!(texts, ["A", "B", "C"]);
}

/// 幻灯片总数超过 `max_slides` 返回类型化错误(不 panic);恰等于上限则通过。
#[test]
fn slide_count_over_limit_is_typed_error() {
    let bytes = deck(&["a", "b", "c", "d"], &[0, 1, 2, 3]);
    let tight = ZipLimits {
        max_slides: 3,
        ..ZipLimits::default()
    };
    let (limit, actual) = expect_limit(parse_bytes_with_limits(&bytes, &tight), LimitKind::Slides);
    assert_eq!((limit, actual), (3, 4));
    let exact = ZipLimits {
        max_slides: 4,
        ..ZipLimits::default()
    };
    assert!(parse_bytes_with_limits(&bytes, &exact).is_ok());
}
