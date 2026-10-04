//! "大而合法"的基准文稿:缺省限额必须对它们**零截断、零诊断**(个数预算只防病态输入,内存上界
//! 由模型字节预算负责)。生成器的规模就是定缺省值时实测的规模;四个基准串行跑(互斥锁),
//! 一次只有一份大模型在内存里。
//!
//! 实测(`ParseReport` 用量,调试构建,2026-10):
//! - 500 页 × 每页标题 + 50×20 表格:形状 1 000、节点 1 536 500、模型字节约 0.80 GB;
//! - 2 000 页 × 每页 100 个文本框:形状 200 000、节点 400 000、模型字节约 0.39 GB;
//! - 100 页 × 每页 20 张中等图表(各 4 系列 × 50 点,独立部件):形状 2 000、图表点 500 000;
//! - 带复杂母版的模板(母版 150 + 10 个版式各 30 个装饰形状,300 页各 10 个形状):形状 3 450。

use std::io::{Cursor, Write};
use std::sync::Mutex;

use ppt_parse::{parse_bytes, ParsedPptx, ZipLimits};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

/// 大模型串行:同一时刻只解析一份基准。
static SERIAL: Mutex<()> = Mutex::new(());

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const CHART: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

fn rels(items: &[Rel]) -> String {
    let body: String = items
        .iter()
        .map(|(id, ty, t)| format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{t}"/>"#))
        .collect();
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
    )
}

/// 关系条目 `(Id, Type, Target)`。
type Rel = (String, String, String);

/// 一份包:`slides[i] = (spTree, rels)`,`extra` 为其它部件。
fn pack(slides: Vec<(String, Vec<Rel>)>, extra: Vec<(String, String)>) -> Vec<u8> {
    let n = slides.len();
    let ids: String = (1..=n)
        .map(|i| format!(r#"<p:sldId id="{}" r:id="rId{i}"/>"#, 255 + i))
        .collect();
    let pres_rels: Vec<_> = (1..=n)
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
        for (i, (tree, r)) in slides.into_iter().enumerate() {
            let k = i + 1;
            put(
                &format!("ppt/slides/slide{k}.xml"),
                &format!(r#"<p:sld {NS}><p:cSld><p:spTree>{tree}</p:spTree></p:cSld></p:sld>"#),
            );
            put(&format!("ppt/slides/_rels/slide{k}.xml.rels"), &rels(&r));
        }
        for (name, body) in extra {
            put(&name, &body);
        }
        zip.finish().unwrap();
    }
    buf.into_inner()
}

fn text_box(id: usize, x: i64, y: i64, text: &str, title: bool) -> String {
    let ph = if title { r#"<p:ph type="title"/>"# } else { "" };
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="T{id}"/><p:cNvSpPr txBox="1"/><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="800000" cy="300000"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:rPr lang="en-US" sz="1200"/><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"#
    )
}

/// 500 页 × 每页标题 + 50 行 × 20 列表格(每格一段一 run)。
pub fn table_train(pages: usize) -> Vec<u8> {
    let grid = r#"<a:gridCol w="400000"/>"#.repeat(20);
    let slides = (0..pages)
        .map(|p| {
            let rows: String = (0..50)
                .map(|r| {
                    let cells: String = (0..20)
                        .map(|c| {
                            format!(
                                r#"<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" sz="1000"/><a:t>r{r}c{c}</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc>"#
                            )
                        })
                        .collect();
                    format!(r#"<a:tr h="100000">{cells}</a:tr>"#)
                })
                .collect();
            let table = format!(
                r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="3" name="Table"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="100000" y="900000"/><a:ext cx="8000000" cy="5000000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr firstRow="1" bandRow="1"/><a:tblGrid>{grid}</a:tblGrid>{rows}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#
            );
            (
                format!("{}{table}", text_box(2, 100_000, 100_000, &format!("TITLE {p}"), true)),
                vec![],
            )
        })
        .collect();
    pack(slides, vec![])
}

/// 2 000 页 × 每页 100 个文本框。
pub fn shape_train(pages: usize) -> Vec<u8> {
    let slides = (0..pages)
        .map(|p| {
            let tree: String = (0..100)
                .map(|i| {
                    text_box(
                        i + 2,
                        (i as i64 % 10) * 900_000,
                        (i as i64 / 10) * 650_000,
                        &format!("p{p} box {i}"),
                        i == 0,
                    )
                })
                .collect();
            (tree, vec![])
        })
        .collect();
    pack(slides, vec![])
}

/// 100 页 × 每页 20 张中等图表(4 系列 × 50 点,每张图表独立部件)。
pub fn chart_deck(pages: usize) -> Vec<u8> {
    let ser = |s: usize| {
        let cats: String = (0..50)
            .map(|j| format!(r#"<c:pt idx="{j}"><c:v>Category {j}</c:v></c:pt>"#))
            .collect();
        let vals: String = (0..50)
            .map(|j| format!(r#"<c:pt idx="{j}"><c:v>{}</c:v></c:pt>"#, j * (s + 1)))
            .collect();
        format!(
            r#"<c:ser><c:idx val="{s}"/><c:tx><c:v>Series {s}</c:v></c:tx><c:cat><c:strLit><c:ptCount val="50"/>{cats}</c:strLit></c:cat><c:val><c:numLit><c:formatCode>General</c:formatCode><c:ptCount val="50"/>{vals}</c:numLit></c:val></c:ser>"#
        )
    };
    let chart = format!(
        r#"<c:chartSpace xmlns:c="{CHART}"><c:chart><c:title><c:tx><c:rich><a:p xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:r><a:t>Quarterly</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}</c:barChart></c:plotArea><c:legend/></c:chart></c:chartSpace>"#,
        (0..4).map(ser).collect::<String>()
    );
    let mut extra = Vec::new();
    let slides = (0..pages)
        .map(|p| {
            let mut tree = text_box(2, 0, 0, &format!("Charts {p}"), true);
            let mut r = Vec::new();
            for k in 0..20 {
                let n = p * 20 + k;
                tree.push_str(&format!(
                    r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="{}" name="C"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="{}" y="{}"/><a:ext cx="1800000" cy="1200000"/></p:xfrm><a:graphic><a:graphicData uri="{CHART}"><c:chart xmlns:c="{CHART}" r:id="rC{k}"/></a:graphicData></a:graphic></p:graphicFrame>"#,
                    k + 3,
                    (k % 5) * 1_800_000,
                    700_000 + (k / 5) * 1_300_000
                ));
                r.push((
                    format!("rC{k}"),
                    format!("{REL}/chart"),
                    format!("../charts/chart{n}.xml"),
                ));
                extra.push((format!("ppt/charts/chart{n}.xml"), chart.clone()));
            }
            (tree, r)
        })
        .collect();
    pack(slides, extra)
}

/// 带复杂母版的模板:母版 150 个装饰形状、10 个版式各 30 个,`pages` 页各 10 个文本框。
pub fn template_deck(pages: usize) -> Vec<u8> {
    let deco = |n: usize, tag: &str| -> String {
        (0..n)
            .map(|i| {
                format!(
                    r#"<p:sp><p:nvSpPr><p:cNvPr id="{}" name="{tag}{i}"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{}" y="0"/><a:ext cx="50000" cy="50000"/></a:xfrm><a:prstGeom prst="rect"/><a:solidFill><a:schemeClr val="accent1"><a:lumMod val="75000"/></a:schemeClr></a:solidFill></p:spPr></p:sp>"#,
                    i + 2,
                    i * 60_000
                )
            })
            .collect()
    };
    let mut extra = vec![(
        "ppt/slideMasters/slideMaster1.xml".to_string(),
        format!(
            r#"<p:sldMaster {NS}><p:cSld><p:spTree>{}</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/></p:sldMaster>"#,
            deco(150, "M")
        ),
    )];
    for l in 1..=10 {
        extra.push((
            format!("ppt/slideLayouts/slideLayout{l}.xml"),
            format!(
                r#"<p:sldLayout {NS}><p:cSld><p:spTree>{}</p:spTree></p:cSld></p:sldLayout>"#,
                deco(30, "L")
            ),
        ));
        extra.push((
            format!("ppt/slideLayouts/_rels/slideLayout{l}.xml.rels"),
            rels(&[(
                "rId1".into(),
                format!("{REL}/slideMaster"),
                "../slideMasters/slideMaster1.xml".into(),
            )]),
        ));
    }
    let slides = (0..pages)
        .map(|p| {
            let tree: String = (0..10)
                .map(|i| {
                    text_box(
                        i + 2,
                        0,
                        i as i64 * 600_000,
                        &format!("p{p} line {i}"),
                        i == 0,
                    )
                })
                .collect();
            (
                tree,
                vec![(
                    "rIdL".into(),
                    format!("{REL}/slideLayout"),
                    format!("../slideLayouts/slideLayout{}.xml", p % 10 + 1),
                )],
            )
        })
        .collect();
    pack(slides, extra)
}

fn assert_clean(name: &str, p: &ParsedPptx) {
    assert!(
        p.presentation.diagnostics.is_empty(),
        "{name}: {:?}",
        &p.presentation.diagnostics[..p.presentation.diagnostics.len().min(5)]
    );
    assert!(!p.presentation.report.truncated(), "{name}");
}

/// 缺省值至少是基准实测需求的若干倍(个数预算只是第二道闸)。
fn assert_headroom(name: &str, p: &ParsedPptx, factor: usize) {
    let d = ZipLimits::default();
    let r = &p.presentation.report;
    assert!(
        r.shapes_used * factor <= d.max_total_shapes,
        "{name}: shapes {}",
        r.shapes_used
    );
    assert!(
        r.items_used * factor <= d.max_total_items,
        "{name}: items {}",
        r.items_used
    );
    eprintln!(
        "{name}: shapes {} items {} model_bytes {}",
        r.shapes_used, r.items_used, r.model_bytes
    );
}

#[test]
fn table_train_500_pages_is_complete_under_defaults() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let p = parse_bytes(&table_train(500)).unwrap();
    assert_clean("table-train", &p);
    assert_headroom("table-train", &p, 4);
    assert_eq!(p.presentation.slides.len(), 500);
    let last = &p.presentation.slides[499];
    assert!(ppt_core::export::slide_text(last).contains("TITLE 499"));
    assert!(ppt_core::export::slide_text(last).contains("r49c19"));
    let d = ZipLimits::default();
    assert!(p.presentation.report.model_bytes * 2 <= d.max_model_bytes);
}

#[test]
fn shape_train_2000_pages_is_complete_under_defaults() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let p = parse_bytes(&shape_train(2_000)).unwrap();
    assert_clean("shape-train", &p);
    assert_headroom("shape-train", &p, 4);
    assert_eq!(p.presentation.slides[1999].shapes.len(), 100);
}

#[test]
fn chart_deck_is_complete_under_defaults() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let p = parse_bytes(&chart_deck(100)).unwrap();
    assert_clean("charts", &p);
    assert_headroom("charts", &p, 4);
    let charts = p
        .presentation
        .slides
        .iter()
        .flat_map(|s| &s.shapes)
        .filter(|s| matches!(s, ppt_core::model::Shape::Placeholder(g) if g.chart.is_some()))
        .count();
    assert_eq!(charts, 2_000);
}

#[test]
fn template_deck_is_complete_under_defaults() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let p = parse_bytes(&template_deck(300)).unwrap();
    assert_clean("template", &p);
    assert_headroom("template", &p, 4);
    let r = ppt_parse::resolve(&p);
    assert_eq!(r.inherited_dropped, 0);
    assert_eq!(
        r.slides
            .iter()
            .map(|s| s.inherited_shapes.len())
            .sum::<usize>(),
        300 * 180
    );
}
