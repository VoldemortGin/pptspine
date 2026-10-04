//! SmartArt 后处理:slide 解析出的 SmartArt 占位只带 `dgm:relIds@r:dm`,这里经该 slide
//! 部件的 rels 读 data 部件,并按优先级回填:
//!
//! 1. 有 drawing 部件(`ppt/diagrams/drawingN.xml`,PowerPoint 预渲染的 `dsp:spTree`):
//!    复用形状解析得到一组形状,包成一个以 frame 矩形为外框的组合(drawing 坐标系以 frame
//!    左上为原点、与 frame 同尺寸 => `chOff = 0`、`chExt = frame.ext`),文字导出与 PDF
//!    渲染自然生效。drawing 解析不出任何形状视为畸形,回落到 2。
//! 2. 退回 data 部件:内容点文字写入 [`GraphicPlaceholder::diagram_text`],占位框保留。
//! 3. 两者皆缺:占位框原样保留。

use std::collections::BTreeMap;

use ppt_core::geom::Rect;
use ppt_core::model::{GraphicPlaceholder, GroupShape, Shape};
use ppt_core::DiagnosticKind;

use crate::links::resolve_part_path;
use crate::xml::diagram::{parse_data, DiagramData};
use crate::xml::Relationship;
use crate::zip_pkg::Package;

// 测试用:本线程内 drawing 部件被解析的次数(断言"同一部件只解析一次")。
#[cfg(test)]
thread_local! {
    pub(crate) static DRAWING_PARSE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// 已解析 SmartArt 缓存:键为部件路径。同一部件被多个 frame(或多张 slide)引用时只解析一次,
/// 每个 frame 克隆一份结果。drawing 解析不出形状 / 部件缺失为 `None`。
#[derive(Default)]
pub(crate) struct DiagramCache {
    data: BTreeMap<String, Option<DiagramData>>,
    drawings: BTreeMap<String, Option<Vec<Shape>>>,
}

/// 回填一棵形状树里所有 SmartArt 占位(含组合内)。
pub(crate) fn resolve_diagrams(
    shapes: &mut [Shape],
    rels: &BTreeMap<String, Relationship>,
    part: &str,
    pkg: &Package,
    media_index: &BTreeMap<String, usize>,
    cache: &mut DiagramCache,
) {
    for sh in shapes {
        match sh {
            Shape::Placeholder(gp) if gp.diagram_rel_id.is_some() => {
                if let Some(group) = fill_diagram(gp, rels, part, pkg, media_index, cache) {
                    *sh = Shape::Group(group);
                }
            }
            Shape::Group(g) => {
                resolve_diagrams(&mut g.children, rels, part, pkg, media_index, cache)
            }
            _ => {}
        }
    }
}

/// 处理一个 SmartArt 占位:drawing 可用返回替换它的组合,否则(必要时)写入 data 文字返回 `None`。
fn fill_diagram(
    gp: &mut GraphicPlaceholder,
    rels: &BTreeMap<String, Relationship>,
    part: &str,
    pkg: &Package,
    media_index: &BTreeMap<String, usize>,
    cache: &mut DiagramCache,
) -> Option<GroupShape> {
    // 没有可用 drawing 即降级(占位框 + data 文字):记一条 `SmartArtDegraded`(`part` = data 部件;
    // 关系都找不到时退回源 slide 部件)。
    let Some(rel) = gp.diagram_rel_id.as_deref().and_then(|id| rels.get(id)) else {
        pkg.note(DiagnosticKind::SmartArtDegraded, part, 1);
        return None;
    };
    let data_path = resolve_part_path(part, &rel.target);
    let data = cache
        .data
        .entry(data_path.clone())
        .or_insert_with(|| pkg.part_str(&data_path).map(|x| parse_data(&x)))
        .clone();
    let Some(data) = data else {
        pkg.note(DiagnosticKind::SmartArtDegraded, &data_path, 1);
        return None;
    };

    if let Some(drawing_path) = drawing_path_for(&data, &data_path, rels, part) {
        let children = cache
            .drawings
            .entry(drawing_path.clone())
            .or_insert_with(|| parse_drawing(pkg, &drawing_path, media_index))
            .clone();
        if let Some(children) = children {
            let child_rect = gp.rect.map(|r| Rect::new(0, 0, r.w, r.h));
            return Some(GroupShape {
                rect: gp.rect,
                child_rect,
                children,
                ..GroupShape::default()
            });
        }
    }
    pkg.note(DiagnosticKind::SmartArtDegraded, &data_path, 1);
    gp.diagram_text = data.texts;
    None
}

/// drawing 部件路径:优先 data 部件 `dataModelExt@relId`(指向 slide rels);缺失时在 slide rels
/// 里找 `diagramDrawing` 关系中文件名编号与 data 部件一致的(`data3.xml` ↔ `drawing3.xml`)。
fn drawing_path_for(
    data: &DiagramData,
    data_path: &str,
    rels: &BTreeMap<String, Relationship>,
    part: &str,
) -> Option<String> {
    if let Some(rel) = data.drawing_rel_id.as_deref().and_then(|id| rels.get(id)) {
        return Some(resolve_part_path(part, &rel.target));
    }
    let number = |path: &str| -> String {
        path.rsplit('/')
            .next()
            .unwrap_or("")
            .chars()
            .filter(char::is_ascii_digit)
            .collect()
    };
    let want = number(data_path);
    rels.values()
        .filter(|r| r.rel_type.ends_with("/diagramDrawing"))
        .map(|r| resolve_part_path(part, &r.target))
        .find(|p| number(p) == want)
}

/// 读 drawing 部件:复用 slide 形状解析(`dsp:` 与 `p:` 同构,解析按本地名匹配)。
/// 部件缺失 / XML 不良构(截断、标签错配)/ 一个形状都解析不出,均视为畸形,返回 `None`。
fn parse_drawing(
    pkg: &Package,
    path: &str,
    media_index: &BTreeMap<String, usize>,
) -> Option<Vec<Shape>> {
    let xml_text = pkg.part_str(path)?;
    if pkg.is_malformed(path) {
        return None;
    }
    #[cfg(test)]
    DRAWING_PARSE_COUNT.with(|c| c.set(c.get() + 1));
    let rels_xml = pkg.slide_rels_str(path);
    let shapes =
        crate::parse_shape_part(pkg, path, &xml_text, rels_xml.as_deref(), media_index).shapes;
    (!shapes.is_empty()).then_some(shapes)
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use ppt_core::model::Shape;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    use super::DRAWING_PARSE_COUNT;
    use crate::parse_bytes;

    const DGM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";

    fn frame() -> String {
        format!(
            r#"<p:graphicFrame><p:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></p:xfrm>
               <a:graphic><a:graphicData uri="{DGM}"><dgm:relIds xmlns:dgm="{DGM}" r:dm="rId2"/>
               </a:graphicData></a:graphic></p:graphicFrame>"#
        )
    }

    /// 两个 frame 引用同一 data / drawing 部件 => drawing 只解析一次,两个 frame 都得到组合。
    #[test]
    fn shared_drawing_part_is_parsed_once() {
        let ns = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
            xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
            xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;
        let rels = |body: &str| {
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
            )
        };
        let parts = [
            (
                "ppt/presentation.xml",
                format!(r#"<p:presentation {ns}><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#),
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                rels(r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>"#),
            ),
            (
                "ppt/slides/slide1.xml",
                format!(r#"<p:sld {ns}><p:cSld><p:spTree>{0}{0}</p:spTree></p:cSld></p:sld>"#, frame()),
            ),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                rels(r#"<Relationship Id="rId2" Type="x/diagramData" Target="../diagrams/data1.xml"/>
                    <Relationship Id="rId6" Type="http://schemas.microsoft.com/office/2007/relationships/diagramDrawing" Target="../diagrams/drawing1.xml"/>"#),
            ),
            (
                "ppt/diagrams/data1.xml",
                format!(r#"<dgm:dataModel xmlns:dgm="{DGM}"><dgm:ptLst/></dgm:dataModel>"#),
            ),
            (
                "ppt/diagrams/drawing1.xml",
                r#"<dsp:drawing xmlns:dsp="urn:dsp" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><dsp:spTree>
                   <dsp:sp><dsp:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="10" cy="10"/></a:xfrm></dsp:spPr>
                   <dsp:txBody><a:p><a:r><a:t>X</a:t></a:r></a:p></dsp:txBody></dsp:sp></dsp:spTree></dsp:drawing>"#
                    .to_string(),
            ),
        ];
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buf);
            for (name, body) in &parts {
                zip.start_file(*name, SimpleFileOptions::default()).unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        DRAWING_PARSE_COUNT.with(|c| c.set(0));
        let parsed = parse_bytes(&buf.into_inner()).unwrap();
        let shapes = &parsed.presentation.slides[0].shapes;
        assert_eq!(shapes.len(), 2);
        assert!(shapes.iter().all(|s| matches!(s, Shape::Group(_))));
        assert_eq!(DRAWING_PARSE_COUNT.with(|c| c.get()), 1);
    }
}
