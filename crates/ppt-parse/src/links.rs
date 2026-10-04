//! 超链接后处理:slide 解析出的 [`Hyperlink`] 只带 `r:id` / `action`,这里经该 slide 部件的
//! rels 回填外链目标(`url`),并把内部跳转(`ppaction://hlinksldjump` 经 rels 指向的 slide
//! 部件、`ppaction://hlinkshowjump?jump=…` 的首/末/上/下一页)折成目标幻灯片序号。
//!
//! 放在解析全部 slide 之后做:跳转目标需要"部件路径 → 幻灯片序号"的全量映射。

use std::collections::BTreeMap;

use ppt_core::model::{Hyperlink, Paragraph, Shape};

use crate::xml::Relationship;

/// 内部跳转动作前缀(PowerPoint 动作 URI)。
const PPACTION: &str = "ppaction://";

/// 一张 slide 的链接解析上下文。
pub(crate) struct LinkCtx<'a> {
    /// 该 slide 部件的 rels。
    pub rels: &'a BTreeMap<String, Relationship>,
    /// 该 slide 的部件路径(如 `ppt/slides/slide2.xml`),用于解析相对 `Target`。
    pub part: &'a str,
    /// 部件路径 → 幻灯片序号。
    pub part_index: &'a BTreeMap<String, usize>,
    /// 当前幻灯片序号。
    pub current: usize,
    /// 幻灯片总数。
    pub count: usize,
}

/// 回填一棵形状树里所有超链接(形状级 + run 级,含组合 / 表格单元格)。
pub(crate) fn resolve_links(shapes: &mut [Shape], ctx: &LinkCtx) {
    for sh in shapes {
        match sh {
            Shape::TextBox(tf) => {
                fill(tf.hyperlink.as_mut(), ctx);
                paragraphs(&mut tf.paragraphs, ctx);
            }
            Shape::Auto(a) => {
                fill(a.hyperlink.as_mut(), ctx);
                if let Some(tf) = a.text.as_mut() {
                    fill(tf.hyperlink.as_mut(), ctx);
                    paragraphs(&mut tf.paragraphs, ctx);
                }
            }
            Shape::Picture(p) => fill(p.hyperlink.as_mut(), ctx),
            Shape::Table(t) => {
                for cell in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()) {
                    paragraphs(&mut cell.paragraphs, ctx);
                }
            }
            Shape::Group(g) => resolve_links(&mut g.children, ctx),
            Shape::Connector(_) | Shape::Placeholder(_) => {}
        }
    }
}

fn paragraphs(paras: &mut [Paragraph], ctx: &LinkCtx) {
    for run in paras.iter_mut().flat_map(|p| p.runs.iter_mut()) {
        fill(run.hyperlink.as_mut(), ctx);
    }
}

fn fill(link: Option<&mut Hyperlink>, ctx: &LinkCtx) {
    let Some(link) = link else {
        return;
    };
    let rel = link.rel_id.as_deref().and_then(|id| ctx.rels.get(id));
    match link.action.as_deref() {
        Some(a) if a.starts_with(PPACTION) => {
            link.slide_index = if a.starts_with("ppaction://hlinksldjump") {
                rel.map(|r| resolve_part_path(ctx.part, &r.target))
                    .and_then(|p| ctx.part_index.get(&p).copied())
            } else if let Some(jump) = a.strip_prefix("ppaction://hlinkshowjump?jump=") {
                show_jump(jump, ctx.current, ctx.count)
            } else {
                None
            };
        }
        // 无动作(或非 ppaction 动作)= 普通超链接:目标即 rels 的 Target。
        _ => link.url = rel.map(|r| r.target.clone()).filter(|t| !t.is_empty()),
    }
}

/// `hlinkshowjump` 的相对跳转 → 目标序号(越界为 `None`)。
fn show_jump(jump: &str, current: usize, count: usize) -> Option<usize> {
    let target = match jump {
        "firstslide" => Some(0),
        "lastslide" => count.checked_sub(1),
        "nextslide" => current.checked_add(1),
        "previousslide" => current.checked_sub(1),
        _ => None,
    }?;
    (target < count).then_some(target)
}

/// 把相对某部件的 rels `Target` 解析成包内绝对部件路径(处理 `../` 与 `./`;
/// 以 `/` 开头的视为包根绝对路径;`base_part` 为空串表示相对包根)。
/// `..` 越出包根的 Target 被**拒绝**:返回空串(包里没有叫 `""` 的部件,取不到任何东西),
/// 绝不把多出的 `..` 钳在根上去碰别的部件。
pub(crate) fn resolve_part_path(base_part: &str, target: &str) -> String {
    let (mut segs, rel): (Vec<&str>, &str) = match target.strip_prefix('/') {
        Some(abs) => (Vec::new(), abs),
        None => {
            let mut dir: Vec<&str> = base_part.split('/').filter(|s| !s.is_empty()).collect();
            dir.pop(); // 去掉部件文件名,留目录。
            (dir, target)
        }
    };
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if segs.pop().is_none() {
                    return String::new();
                }
            }
            s => segs.push(s),
        }
    }
    segs.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_paths_resolve_relative_to_slide_dir() {
        assert_eq!(
            resolve_part_path("ppt/slides/slide1.xml", "slide3.xml"),
            "ppt/slides/slide3.xml"
        );
        assert_eq!(
            resolve_part_path("ppt/slides/slide1.xml", "../slides/slide2.xml"),
            "ppt/slides/slide2.xml"
        );
        assert_eq!(
            resolve_part_path("ppt/slides/slide1.xml", "/ppt/slides/slide4.xml"),
            "ppt/slides/slide4.xml"
        );
    }

    #[test]
    fn part_paths_escaping_the_package_root_are_rejected() {
        assert_eq!(
            resolve_part_path("ppt/slides/slide1.xml", "../../x.xml"),
            "x.xml"
        );
        assert_eq!(
            resolve_part_path("ppt/slides/slide1.xml", "../../../x.xml"),
            ""
        );
        assert_eq!(resolve_part_path("ppt/presentation.xml", "../../x.xml"), "");
        assert_eq!(resolve_part_path("", "/../x.xml"), "");
        assert_eq!(
            resolve_part_path("", "ppt/./presentation.xml"),
            "ppt/presentation.xml"
        );
        assert_eq!(resolve_part_path("", &"../".repeat(500)), "");
    }

    #[test]
    fn show_jumps_are_relative_and_bounded() {
        assert_eq!(show_jump("nextslide", 1, 3), Some(2));
        assert_eq!(show_jump("nextslide", 2, 3), None);
        assert_eq!(show_jump("previousslide", 0, 3), None);
        assert_eq!(show_jump("lastslide", 0, 3), Some(2));
        assert_eq!(show_jump("firstslide", 2, 3), Some(0));
        assert_eq!(show_jump("endshow", 0, 3), None);
    }
}
