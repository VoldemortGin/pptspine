//! 图表后处理:slide 解析出的图表占位只带 `c:chart@r:id`,这里经该 slide 部件的 rels
//! 找到 `ppt/charts/chartN.xml`,解析其缓存数据回填 [`GraphicPlaceholder::chart`]。

use std::collections::BTreeMap;

use ppt_core::model::{Chart, GraphicPlaceholder, Shape};

use crate::links::resolve_part_path;
use crate::xml::{self, Relationship};
use crate::zip_pkg::Package;

// 测试用:本线程内 `xml::chart::parse` 被调用的次数(断言"同一部件只解析一次")。
#[cfg(test)]
thread_local! {
    static PARSE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// 已解析图表缓存:键为解析后的图表部件路径,值为解析结果(部件缺失为 `None`)。
/// 同一部件被多个 graphicFrame(或多张 slide)引用时只解析一次,每个 frame 克隆一份结果。
pub(crate) type ChartCache = BTreeMap<String, Option<Chart>>;

/// 回填一棵形状树里所有图表占位(含组合内)。部件缺失时 `chart` 保持 `None`。
pub(crate) fn resolve_charts(
    shapes: &mut [Shape],
    rels: &BTreeMap<String, Relationship>,
    part: &str,
    pkg: &Package,
    cache: &mut ChartCache,
) {
    for sh in shapes {
        match sh {
            Shape::Placeholder(GraphicPlaceholder {
                chart_rel_id: Some(id),
                chart,
                ..
            }) => {
                let Some(rel) = rels.get(id.as_str()) else {
                    *chart = None;
                    continue;
                };
                let path = resolve_part_path(part, &rel.target);
                *chart = cache
                    .entry(path)
                    .or_insert_with_key(|path| {
                        pkg.part_str(path).map(|x| {
                            #[cfg(test)]
                            PARSE_COUNT.with(|c| c.set(c.get() + 1));
                            xml::chart::parse(&x)
                        })
                    })
                    .clone();
            }
            Shape::Group(g) => resolve_charts(&mut g.children, rels, part, pkg, cache),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use ppt_core::model::{GraphicPlaceholder, Shape};
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    use super::PARSE_COUNT;
    use crate::parse_bytes;

    const REL_CHART: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart";

    fn frame(rid: &str) -> String {
        format!(
            r#"<p:graphicFrame><p:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></p:xfrm>
               <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">
               <c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="{rid}"/>
               </a:graphicData></a:graphic></p:graphicFrame>"#
        )
    }

    /// 单 slide、`rids.len()` 个图表 frame;`rid -> chartN.xml` 由 `targets` 给出。
    fn deck(rids: &[&str], targets: &[(&str, &str)]) -> Vec<u8> {
        let frames: String = rids.iter().map(|r| frame(r)).collect();
        let slide = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>{frames}</p:spTree></p:cSld></p:sld>"#
        );
        let slide_rels: String = targets
            .iter()
            .map(|(id, t)| format!(r#"<Relationship Id="{id}" Type="{REL_CHART}" Target="{t}"/>"#))
            .collect();
        let slide_rels = format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{slide_rels}</Relationships>"#
        );
        let pres = r#"<p:presentation xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
            xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
            <p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#;
        let pres_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
            <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#;
        let chart = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:ser><c:tx><c:v>S</c:v></c:tx>
            <c:cat><c:strLit><c:ptCount val="1"/><c:pt idx="0"><c:v>a</c:v></c:pt></c:strLit></c:cat>
            <c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buf);
            let opts = SimpleFileOptions::default();
            let mut put = |name: &str, body: &str| {
                zip.start_file(name, opts).expect("start_file");
                zip.write_all(body.as_bytes()).expect("write");
            };
            put("ppt/presentation.xml", pres);
            put("ppt/_rels/presentation.xml.rels", pres_rels);
            put("ppt/slides/slide1.xml", &slide);
            put("ppt/slides/_rels/slide1.xml.rels", &slide_rels);
            put("ppt/charts/chart1.xml", chart);
            put("ppt/charts/chart2.xml", chart);
            zip.finish().expect("finish zip");
        }
        buf.into_inner()
    }

    fn charts_of(bytes: &[u8]) -> Vec<bool> {
        let parsed = parse_bytes(bytes).expect("parse");
        parsed.presentation.slides[0]
            .shapes
            .iter()
            .map(|s| match s {
                Shape::Placeholder(GraphicPlaceholder { chart, .. }) => chart.is_some(),
                other => panic!("expected placeholder, got {other:?}"),
            })
            .collect()
    }

    /// 同一图表部件被 100 个 graphicFrame 引用:只解析一次,每个 frame 都拿到图表。
    #[test]
    fn same_chart_part_referenced_by_100_frames_is_parsed_once() {
        PARSE_COUNT.with(|c| c.set(0));
        let rids = vec!["rId1"; 100];
        let have = charts_of(&deck(&rids, &[("rId1", "../charts/chart1.xml")]));
        assert_eq!(have.len(), 100);
        assert!(have.iter().all(|&b| b));
        assert_eq!(PARSE_COUNT.with(|c| c.get()), 1);
    }

    /// 不同 rId 指向同一部件也只解析一次;指向不同部件各解析一次。
    #[test]
    fn cache_is_keyed_by_part_path_not_rel_id() {
        PARSE_COUNT.with(|c| c.set(0));
        let have = charts_of(&deck(
            &["rId1", "rId2", "rId3"],
            &[
                ("rId1", "../charts/chart1.xml"),
                ("rId2", "../charts/chart1.xml"),
                ("rId3", "../charts/chart2.xml"),
            ],
        ));
        assert!(have.iter().all(|&b| b));
        assert_eq!(PARSE_COUNT.with(|c| c.get()), 2);
    }
}
