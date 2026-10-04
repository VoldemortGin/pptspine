//! 页脚 / 页码 / 日期占位符与 `a:fld` 求值验收(PDF 渲染前的终态 IR 层):
//! `slidenum` = 放映序号 + `firstSlideNum` − 1(隐藏页仍占页码);`datetime*` 保持字段缓存
//! 文本(绝不取系统时间);版式 / 母版上的 `sldNum` / `ftr` / `dt` 占位符只有幻灯片上实例化
//! 才出现;位置 / 样式沿占位符继承链取,页脚文字取幻灯片自身。pptx 现场合成。

use std::io::{Cursor, Write};

use ppt_core::export::{presentation_markdown_with, presentation_text_with, ExportOptions};
use ppt_core::geom::Rect;
use ppt_core::resolved::{ResolvedPresentation, ResolvedShape};
use ppt_parse::{parse_bytes, resolve};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// 版式上各占位符的位置(`ftr` / `sldNum` / `dt`),与母版不同,验证"版式优先"。
const LAYOUT_SLDNUM: Rect = Rect::new(8_000_000, 6_300_000, 900_000, 400_000);
const LAYOUT_FTR: Rect = Rect::new(3_000_000, 6_300_000, 3_000_000, 400_000);
const LAYOUT_DT: Rect = Rect::new(500_000, 6_300_000, 2_000_000, 400_000);

fn rels(entries: &[(&str, &str, &str)]) -> String {
    let body: String = entries
        .iter()
        .map(|(id, ty, t)| format!(r#"<Relationship Id="{id}" Type="{REL}/{ty}" Target="{t}"/>"#))
        .collect();
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
    )
}

fn ph_sp(ty: &str, idx: u32, rect: Option<Rect>, body: &str, lst: &str) -> String {
    let xfrm = rect
        .map(|r| {
            format!(
                r#"<a:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></a:xfrm>"#,
                r.x, r.y, r.w, r.h
            )
        })
        .unwrap_or_default();
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{idx}" name="{ty}"/><p:cNvSpPr/><p:nvPr><p:ph type="{ty}" idx="{idx}"/></p:nvPr></p:nvSpPr>
<p:spPr>{xfrm}</p:spPr><p:txBody><a:bodyPr/><a:lstStyle>{lst}</a:lstStyle>{body}</p:txBody></p:sp>"#
    )
}

fn fld(ty: &str, cached: &str) -> String {
    format!(
        r#"<a:p><a:fld id="{{F}}" type="{ty}"><a:rPr lang="en-US"/><a:t>{cached}</a:t></a:fld></a:p>"#
    )
}

fn text_p(t: &str) -> String {
    format!(r#"<a:p><a:r><a:t>{t}</a:t></a:r></a:p>"#)
}

/// master / layout 上的三种页脚占位符,文字是"模板提示"(绝不应出现在渲染里)。
fn template_placeholders(layout: bool) -> String {
    let (dt, ftr, num) = if layout {
        (Some(LAYOUT_DT), Some(LAYOUT_FTR), Some(LAYOUT_SLDNUM))
    } else {
        (None, None, None)
    };
    let master_geo = |r: Rect| Some(r);
    let dt = dt.or_else(|| master_geo(Rect::new(1, 1, 1, 1)));
    let ftr = ftr.or_else(|| master_geo(Rect::new(2, 2, 2, 2)));
    let num = num.or_else(|| master_geo(Rect::new(3, 3, 3, 3)));
    let size = r#"<a:lvl1pPr><a:defRPr sz="1200"/></a:lvl1pPr>"#;
    [
        ph_sp(
            "dt",
            10,
            dt,
            &fld("datetimeFigureOut", "TEMPLATE-DATE"),
            size,
        ),
        ph_sp("ftr", 11, ftr, &text_p("TEMPLATE-FOOTER"), size),
        ph_sp("sldNum", 12, num, &fld("slidenum", "TEMPLATE-NUM"), size),
    ]
    .concat()
}

struct SlideDef {
    sp_tree: String,
    hidden: bool,
}

fn deck(pres_attrs: &str, slides: &[SlideDef]) -> ResolvedDeck {
    let ids: String = (1..=slides.len())
        .map(|i| format!(r#"<p:sldId id="{}" r:id="rId{i}"/>"#, 255 + i))
        .collect();
    let pres_rels: Vec<(String, &str, String)> = (1..=slides.len())
        .map(|i| (format!("rId{i}"), "slide", format!("slides/slide{i}.xml")))
        .collect();
    let pres_rels_ref: Vec<(&str, &str, &str)> = pres_rels
        .iter()
        .map(|(a, b, c)| (a.as_str(), *b, c.as_str()))
        .collect();
    let mut parts: Vec<(String, String)> = vec![
        (
            "ppt/presentation.xml".into(),
            format!(
                r#"<p:presentation {NS} {pres_attrs}><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#
            ),
        ),
        (
            "ppt/_rels/presentation.xml.rels".into(),
            rels(&pres_rels_ref),
        ),
        (
            "ppt/slideMasters/slideMaster1.xml".into(),
            format!(
                r#"<p:sldMaster {NS}><p:cSld><p:spTree>{}</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/></p:sldMaster>"#,
                template_placeholders(false)
            ),
        ),
        (
            "ppt/slideMasters/_rels/slideMaster1.xml.rels".into(),
            rels(&[]),
        ),
        (
            "ppt/slideLayouts/slideLayout1.xml".into(),
            format!(
                r#"<p:sldLayout {NS}><p:cSld><p:spTree>{}</p:spTree></p:cSld></p:sldLayout>"#,
                template_placeholders(true)
            ),
        ),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels".into(),
            rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]),
        ),
    ];
    for (i, s) in slides.iter().enumerate() {
        let n = i + 1;
        let show = if s.hidden { r#" show="0""# } else { "" };
        parts.push((
            format!("ppt/slides/slide{n}.xml"),
            format!(
                r#"<p:sld {NS}{show}><p:cSld><p:spTree>{}</p:spTree></p:cSld></p:sld>"#,
                s.sp_tree
            ),
        ));
        parts.push((
            format!("ppt/slides/_rels/slide{n}.xml.rels"),
            rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml")]),
        ));
    }
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut buf);
        for (name, body) in &parts {
            zip.start_file(name.as_str(), SimpleFileOptions::default())
                .unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    let parsed = parse_bytes(&buf.into_inner()).expect("parse");
    let resolved = resolve(&parsed);
    ResolvedDeck { parsed, resolved }
}

struct ResolvedDeck {
    parsed: ppt_parse::ParsedPptx,
    resolved: ResolvedPresentation,
}

impl ResolvedDeck {
    /// 第 `i` 张 slide 上所有文本形状的 `(rect, run 文本, run 字号)`。
    fn texts(&self, i: usize) -> Vec<(Option<Rect>, String, f64)> {
        self.resolved.slides[i]
            .shapes
            .iter()
            .filter_map(|s| match s {
                ResolvedShape::TextBox(t) => Some(t),
                ResolvedShape::Auto(a) => a.text.as_ref(),
                _ => None,
            })
            .flat_map(|t| {
                t.paragraphs.iter().flat_map(move |p| {
                    p.runs
                        .iter()
                        .map(move |r| (t.rect, r.text.clone(), f64::from(r.size_pt)))
                })
            })
            .collect()
    }
}

fn num_slide(cached: &str) -> SlideDef {
    SlideDef {
        sp_tree: ph_sp("sldNum", 12, None, &fld("slidenum", cached), ""),
        hidden: false,
    }
}

#[test]
fn slide_number_field_evaluates_to_display_number_and_inherits_layout_geometry() {
    // 缓存文本故意是过期 / 提示值,求值必须无视它。
    let d = deck("", &[num_slide("99"), num_slide("‹#›"), num_slide("")]);
    for (i, want) in ["1", "2", "3"].iter().enumerate() {
        let t = d.texts(i);
        assert_eq!(t.len(), 1, "{t:?}");
        assert_eq!(t[0].1, *want);
        // 位置沿占位符链取自版式(idx 12 / sldNum),字号来自该链(母版 / 版式 lstStyle 1200)。
        assert_eq!(t[0].0, Some(LAYOUT_SLDNUM));
        assert_eq!(t[0].2, 12.0);
    }
}

#[test]
fn first_slide_num_shifts_displayed_numbers() {
    let d = deck(
        r#"firstSlideNum="5""#,
        &[num_slide("1"), num_slide("1"), num_slide("1")],
    );
    let got: Vec<String> = (0..3).map(|i| d.texts(i)[0].1.clone()).collect();
    assert_eq!(got, ["5", "6", "7"]);
    let d = deck(r#"firstSlideNum="0""#, &[num_slide("1"), num_slide("1")]);
    assert_eq!(
        (d.texts(0)[0].1.as_str(), d.texts(1)[0].1.as_str()),
        ("0", "1")
    );
    // 非法值回落缺省 1。
    let d = deck(r#"firstSlideNum="abc""#, &[num_slide("x")]);
    assert_eq!(d.texts(0)[0].1, "1");
}

#[test]
fn hidden_slide_still_occupies_a_page_number() {
    let mut hidden = num_slide("9");
    hidden.hidden = true;
    let d = deck("", &[num_slide("9"), hidden, num_slide("9")]);
    // PowerPoint:隐藏页照常占号,放映 / 导出跳过它后页码不重排(1, 3)。
    assert_eq!(d.texts(0)[0].1, "1");
    assert_eq!(d.texts(1)[0].1, "2");
    assert_eq!(d.texts(2)[0].1, "3");
}

#[test]
fn slidenum_field_in_ordinary_text_box_is_evaluated_too() {
    let sp = r#"<p:sp><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>Page </a:t></a:r><a:fld id="{F}" type="slidenum"><a:t>7</a:t></a:fld></a:p></p:txBody></p:sp>"#.to_string();
    let d = deck(
        "",
        &[
            SlideDef {
                sp_tree: sp.clone(),
                hidden: false,
            },
            SlideDef {
                sp_tree: sp,
                hidden: false,
            },
        ],
    );
    let t = d.texts(1);
    assert_eq!(
        t.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(),
        ["Page ", "2"]
    );
}

#[test]
fn datetime_field_keeps_cached_text() {
    let sp = ph_sp("dt", 10, None, &fld("datetime1", "1/2/2020"), "")
        + &ph_sp("dt", 10, None, &fld("datetimeFigureOut", "stale text"), "");
    let d = deck(
        "",
        &[SlideDef {
            sp_tree: sp,
            hidden: false,
        }],
    );
    let got: Vec<String> = d.texts(0).into_iter().map(|t| t.1).collect();
    assert_eq!(got, ["1/2/2020", "stale text"]);
    assert_eq!(d.texts(0)[0].0, Some(LAYOUT_DT));
}

#[test]
fn footer_text_comes_from_the_slide_not_the_layout() {
    let d = deck(
        "",
        &[SlideDef {
            sp_tree: ph_sp("ftr", 11, None, &text_p("Quarterly review"), ""),
            hidden: false,
        }],
    );
    let t = d.texts(0);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].1, "Quarterly review");
    assert_eq!(t[0].0, Some(LAYOUT_FTR));
}

#[test]
fn template_footer_placeholders_are_not_drawn_without_a_slide_instance() {
    let d = deck(
        "",
        &[SlideDef {
            sp_tree: String::new(),
            hidden: false,
        }],
    );
    // 版式 / 母版占位符(含 TEMPLATE-* 提示文字)既不在继承图形里,slide 自身也没有形状。
    assert!(d.resolved.slides[0].inherited_shapes.is_empty());
    assert!(d.resolved.slides[0].shapes.is_empty());
}

/// 文本类导出与 PDF 共用 `resolve.rs` 的字段求值:`slidenum` 输出真实页码(不是缓存文本),
/// `datetime*` 仍是缓存文本。标题(Markdown 标题行)与普通段落两条路径都覆盖。
#[test]
fn text_exports_use_the_evaluated_slide_number_and_keep_cached_dates() {
    let body = r#"<p:sp><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>Page </a:t></a:r><a:fld id="{F}" type="slidenum"><a:t>7</a:t></a:fld><a:r><a:t> on </a:t></a:r><a:fld id="{D}" type="datetime1"><a:t>1/2/2020</a:t></a:fld></a:p></p:txBody></p:sp>"#;
    let title = ph_sp(
        "title",
        20,
        None,
        r#"<a:p><a:r><a:t>Slide </a:t></a:r><a:fld id="{F}" type="slidenum"><a:t>TITLE-STALE</a:t></a:fld></a:p>"#,
        "",
    );
    let sp_tree = format!("{title}{body}");
    let slides = [
        SlideDef {
            sp_tree: sp_tree.clone(),
            hidden: false,
        },
        SlideDef {
            sp_tree,
            hidden: false,
        },
    ];
    let d = deck(r#"firstSlideNum="5""#, &slides);
    let opts = ExportOptions::default();

    let text = presentation_text_with(&d.parsed.presentation, Some(&d.resolved), &opts);
    assert!(text.contains("Page 5 on 1/2/2020"), "{text}");
    assert!(text.contains("Page 6 on 1/2/2020"), "{text}");
    assert!(
        text.contains("Slide 5") && text.contains("Slide 6"),
        "{text}"
    );
    assert!(
        !text.contains("STALE") && !text.contains("Page 7"),
        "{text}"
    );

    let md = presentation_markdown_with(&d.parsed.presentation, Some(&d.resolved), &opts);
    assert!(
        md.contains("### Slide 5") && md.contains("### Slide 6"),
        "{md}"
    );
    assert!(
        md.contains("Page 5 on 1/2/2020") && md.contains("Page 6 on 1/2/2020"),
        "{md}"
    );
    assert!(!md.contains("STALE") && !md.contains("Page 7"), "{md}");

    // 没有终态 IR 时只能给缓存文本(求值在 resolve 里,导出不复制公式)。
    let raw = presentation_text_with(&d.parsed.presentation, None, &opts);
    assert!(raw.contains("Page 7"), "{raw}");
}

/// 表格单元格里的 `slidenum` 字段与正文形状一样:文本导出(纯文本 / GFM / HTML 三种表格
/// 渲染)取终态求值结果,而不是缓存文本;没有终态 IR 时才退回缓存文本。
#[test]
fn table_cell_fields_use_the_evaluated_slide_number_in_text_exports() {
    let cell = |inner: &str, attrs: &str| {
        format!(r#"<a:tc{attrs}><a:txBody><a:bodyPr/>{inner}</a:txBody></a:tc>"#)
    };
    let page = r#"<a:p><a:r><a:t>Page </a:t></a:r><a:fld id="{F}" type="slidenum"><a:t>7</a:t></a:fld></a:p>"#;
    let table = |rows: &str| {
        format!(
            r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="T"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
<p:xfrm><a:off x="0" y="0"/><a:ext cx="200" cy="100"/></p:xfrm>
<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl>
<a:tblGrid><a:gridCol w="100"/><a:gridCol w="100"/></a:tblGrid>{rows}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#
        )
    };
    let plain = table(&format!(
        r#"<a:tr h="100">{}{}</a:tr>"#,
        cell(page, ""),
        cell(r#"<a:p><a:r><a:t>x</a:t></a:r></a:p>"#, "")
    ));
    // 带合并(gridSpan)→ Markdown 走 HTML 表格路径。
    let merged = table(&format!(
        r#"<a:tr h="100">{}{}</a:tr>"#,
        cell(page, r#" gridSpan="2""#),
        cell(r#"<a:p><a:r><a:t>hm</a:t></a:r></a:p>"#, r#" hMerge="1""#)
    ));
    let slides = [
        SlideDef {
            sp_tree: format!("{plain}{merged}"),
            hidden: false,
        },
        SlideDef {
            sp_tree: format!("{plain}{merged}"),
            hidden: false,
        },
    ];
    let d = deck(r#"firstSlideNum="5""#, &slides);
    let opts = ExportOptions::default();

    let text = presentation_text_with(&d.parsed.presentation, Some(&d.resolved), &opts);
    assert!(
        text.contains("Page 5 | x") && text.contains("Page 6 | x"),
        "{text}"
    );
    assert!(!text.contains("Page 7"), "{text}");

    let md = presentation_markdown_with(&d.parsed.presentation, Some(&d.resolved), &opts);
    assert!(md.contains("| Page 5 | x |"), "gfm: {md}");
    assert!(md.contains("<td colspan=\"2\">Page 6</td>"), "html: {md}");
    assert!(!md.contains("Page 7"), "{md}");

    let raw = presentation_text_with(&d.parsed.presentation, None, &opts);
    assert!(raw.contains("Page 7 | x"), "{raw}");
}
