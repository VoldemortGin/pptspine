//! 全文稿预算耗尽时的公平性与诊断如实性:截断应表现为"每页的长尾被截"而不是"后半本书消失";
//! 被截断时诊断 `count` 等于实际丢弃的内容量(含被整体跳过的容器的后代);被截断的部件都能被
//! 调用方识别。

use std::io::{Cursor, Write};

use ppt_core::model::Shape;
use ppt_core::DiagnosticKind;
use ppt_parse::{parse_bytes_with_limits, ParsedPptx, ZipLimits};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

type Rel = (String, String, String);

fn rels(items: &[Rel]) -> String {
    let body: String = items
        .iter()
        .map(|(id, ty, t)| format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{t}"/>"#))
        .collect();
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
    )
}

fn pack(slides: &[(String, Vec<Rel>)], extra: &[(String, String)]) -> Vec<u8> {
    let n = slides.len();
    let ids: String = (1..=n)
        .map(|i| format!(r#"<p:sldId id="{}" r:id="rId{i}"/>"#, 255 + i))
        .collect();
    let pres_rels: Vec<Rel> = (1..=n)
        .map(|i| {
            (
                format!("rId{i}"),
                format!("{REL}/slide"),
                format!("slides/slide{i}.xml"),
            )
        })
        .collect();
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default();
        let mut put = |name: &str, body: &str| {
            zip.start_file(name, opts).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        };
        put(
            "ppt/presentation.xml",
            &format!(
                r#"<p:presentation {NS}><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
            ),
        );
        put("ppt/_rels/presentation.xml.rels", &rels(&pres_rels));
        for (i, (tree, r)) in slides.iter().enumerate() {
            let k = i + 1;
            put(
                &format!("ppt/slides/slide{k}.xml"),
                &format!(r#"<p:sld {NS}><p:cSld><p:spTree>{tree}</p:spTree></p:cSld></p:sld>"#),
            );
            put(&format!("ppt/slides/_rels/slide{k}.xml.rels"), &rels(r));
        }
        for (name, body) in extra {
            put(name, body);
        }
        zip.finish().unwrap();
    }
    buf.into_inner()
}

fn title(text: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"#
    )
}

/// 一页:标题 + `rows` × `cols` 表格(每格一段一 run)。节点需求 = 2 + cols + rows × (1 + 3·cols)。
fn table_slide(i: usize, rows: usize, cols: usize) -> String {
    let grid = r#"<a:gridCol w="1"/>"#.repeat(cols);
    let row = format!(
        "<a:tr h=\"1\">{}</a:tr>",
        "<a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>v</a:t></a:r></a:p></a:txBody></a:tc>"
            .repeat(cols)
    );
    format!(
        r#"{}<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="3" name="G"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="0" y="200"/><a:ext cx="100" cy="100"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblGrid>{grid}</a:tblGrid>{}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#,
        title(&format!("TITLE{i}")),
        row.repeat(rows)
    )
}

/// 模型里实际保留的节点数(段落 / run / 网格列 / 行 / 单元格)。
fn kept_nodes(p: &ParsedPptx) -> usize {
    let paras =
        |ps: &[ppt_core::model::Paragraph]| -> usize { ps.iter().map(|p| 1 + p.runs.len()).sum() };
    p.presentation
        .slides
        .iter()
        .flat_map(|s| &s.shapes)
        .map(|s| match s {
            Shape::TextBox(t) => paras(&t.paragraphs),
            Shape::Auto(a) => a.text.as_ref().map_or(0, |t| paras(&t.paragraphs)),
            Shape::Table(t) => {
                t.col_widths.len()
                    + t.rows
                        .iter()
                        .map(|r| {
                            1 + r
                                .cells
                                .iter()
                                .map(|c| 1 + paras(&c.paragraphs))
                                .sum::<usize>()
                        })
                        .sum::<usize>()
            }
            _ => 0,
        })
        .sum()
}

fn diag_sum(p: &ParsedPptx, kind: DiagnosticKind) -> usize {
    p.presentation
        .diagnostics
        .iter()
        .filter(|d| d.kind == kind)
        .map(|d| d.count)
        .sum()
}

fn table_deck(slides: usize) -> Vec<u8> {
    let s: Vec<_> = (0..slides)
        .map(|i| (table_slide(i, 20, 10), vec![]))
        .collect();
    pack(&s, &[])
}

/// 20 页 × (标题 + 20×10 表格),节点总预算只够约一半:每页都保住标题和表格前部(每页保底
/// `min(2000, 6000/20)` = 300 个节点),而不是前一半完整、后一半整页为空。
#[test]
fn deck_wide_truncation_is_spread_across_pages() {
    let limits = ZipLimits {
        max_total_items: 6_000,
        ..ZipLimits::default()
    };
    let p = parse_bytes_with_limits(&table_deck(20), &limits).unwrap();
    for (i, s) in p.presentation.slides.iter().enumerate() {
        let text = ppt_core::export::slide_text(s);
        assert!(text.contains(&format!("TITLE{i}")), "第 {i} 页标题丢了");
        let Some(Shape::Table(t)) = s.shapes.get(1) else {
            panic!("第 {i} 页表格丢了");
        };
        assert!(t.rows.len() >= 5, "第 {i} 页只剩 {} 行", t.rows.len());
    }
}

/// 诊断计数如实:`content-truncated` 合计 = 文档里的节点需求 − 模型里保留的节点数
/// (被整体跳过的行 / 单元格里的段落与 run 都算上)。
#[test]
fn content_truncated_counts_every_dropped_node() {
    let slides = 20;
    let demand = slides * (2 + 10 + 20 * (1 + 3 * 10));
    let limits = ZipLimits {
        max_total_items: 6_000,
        ..ZipLimits::default()
    };
    let p = parse_bytes_with_limits(&table_deck(slides), &limits).unwrap();
    let kept = kept_nodes(&p);
    assert!(kept < demand);
    assert_eq!(
        diag_sum(&p, DiagnosticKind::ContentTruncated),
        demand - kept
    );
}

/// 形状预算耗尽时被整体跳过的组合:其后代形状与后代里的文字节点都计入丢弃数。
#[test]
fn skipped_groups_count_their_descendants() {
    let tb = r#"<p:sp><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>x</a:t></a:r></a:p><a:p/></p:txBody></p:sp>"#;
    let group = format!(
        r#"<p:grpSp><p:grpSpPr/>{}{tb}</p:grpSp>"#,
        "<p:cxnSp></p:cxnSp>".repeat(9)
    );
    let limits = ZipLimits {
        max_part_shapes: 11, // 第一个组合(1)+ 10 个子形状
        ..ZipLimits::default()
    };
    let p = parse_bytes_with_limits(&pack(&[(group.repeat(3), vec![])], &[]), &limits).unwrap();
    // 后两个组合整体跳过:各 1 + 10 个形状、3 个文字节点(2 段 + 1 run)。
    assert_eq!(diag_sum(&p, DiagnosticKind::ShapesTruncated), 22);
    assert_eq!(diag_sum(&p, DiagnosticKind::ContentTruncated), 6);
}

/// 被所有幻灯片依赖的母版先解析:总预算紧张时母版不再拿到 0,每页也都有保底。
#[test]
fn masters_are_parsed_before_slides_and_slides_keep_a_floor() {
    let shapes = |n: usize| "<p:cxnSp></p:cxnSp>".repeat(n);
    let layout_rel = vec![(
        "rIdL".to_string(),
        format!("{REL}/slideLayout"),
        "../slideLayouts/slideLayout1.xml".to_string(),
    )];
    let slides: Vec<_> = (0..5).map(|_| (shapes(200), layout_rel.clone())).collect();
    let extra = vec![
        (
            "ppt/slideLayouts/slideLayout1.xml".to_string(),
            format!(r#"<p:sldLayout {NS}><p:cSld><p:spTree/></p:cSld></p:sldLayout>"#),
        ),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels".to_string(),
            rels(&[(
                "rId1".into(),
                format!("{REL}/slideMaster"),
                "../slideMasters/slideMaster1.xml".into(),
            )]),
        ),
        (
            "ppt/slideMasters/slideMaster1.xml".to_string(),
            format!(
                r#"<p:sldMaster {NS}><p:cSld><p:spTree>{}</p:spTree></p:cSld></p:sldMaster>"#,
                shapes(100)
            ),
        ),
    ];
    let limits = ZipLimits {
        max_part_shapes: 1_000,
        max_total_shapes: 800,
        ..ZipLimits::default()
    };
    let p = parse_bytes_with_limits(&pack(&slides, &extra), &limits).unwrap();
    let master: usize = p.inherit.masters.values().map(|m| m.shapes.len()).sum();
    assert_eq!(master, 100, "母版完整");
    let per: Vec<usize> = p
        .presentation
        .slides
        .iter()
        .map(|s| s.shapes.len())
        .collect();
    assert!(per.iter().all(|&n| n >= 100), "{per:?}");
    assert_eq!(per.iter().sum::<usize>() + master, 800);
}

/// 被截断的部件全部列在 `report.truncated_parts`,即使诊断条目已超过上限(10 000)而并入汇总。
#[test]
fn report_lists_every_truncated_part_beyond_the_diagnostic_cap() {
    let n = 5_100;
    let tree = r#"<p:sp><p:spPr/><p:txBody><a:bodyPr/><a:p/><a:p/></p:txBody></p:sp>"#;
    let dangling = vec![(
        "rIdX".to_string(),
        format!("{REL}/image"),
        "../media/missing.png".to_string(),
    )];
    let slides: Vec<_> = (0..n)
        .map(|_| (tree.to_string(), dangling.clone()))
        .collect();
    let limits = ZipLimits {
        max_part_items: 1,
        max_entries: 3 * n,
        max_slides: n,
        ..ZipLimits::default()
    };
    let p = parse_bytes_with_limits(&pack(&slides, &[]), &limits).unwrap();
    // 诊断条目被封顶,后面的截断并入了 part="" 的汇总……
    assert!(p
        .presentation
        .diagnostics
        .iter()
        .any(|d| d.kind == DiagnosticKind::ContentTruncated && d.part.is_empty()));
    // ……但汇总里每个被截断的部件都在。
    let r = &p.presentation.report;
    assert!(r.truncated());
    assert_eq!(r.truncated_parts.len(), n);
    assert_eq!(r.dropped_items, n);
    assert!(r
        .truncated_parts
        .contains(&format!("ppt/slides/slide{n}.xml")));
}

/// 完整解析时报告为空(用量照常给出)。
#[test]
fn clean_parse_reports_usage_without_truncation() {
    let p = parse_bytes_with_limits(&table_deck(2), &ZipLimits::default()).unwrap();
    let r = &p.presentation.report;
    assert!(!r.truncated());
    assert_eq!(r.items_used, 2 * (2 + 10 + 20 * 31));
    assert_eq!(r.shapes_used, 4);
    assert!(r.model_bytes > 0);
    assert_eq!(
        r.model_bytes,
        ppt_parse::estimated_model_bytes(&p).max(r.model_bytes)
    );
}
