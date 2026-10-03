//! 图表后处理:slide 解析出的图表占位只带 `c:chart@r:id`,这里经该 slide 部件的 rels
//! 找到 `ppt/charts/chartN.xml`,解析其缓存数据回填 [`GraphicPlaceholder::chart`]。

use std::collections::BTreeMap;

use ppt_core::model::{GraphicPlaceholder, Shape};

use crate::links::resolve_part_path;
use crate::xml::{self, Relationship};
use crate::zip_pkg::Package;

/// 回填一棵形状树里所有图表占位(含组合内)。部件缺失时 `chart` 保持 `None`。
pub(crate) fn resolve_charts(
    shapes: &mut [Shape],
    rels: &BTreeMap<String, Relationship>,
    part: &str,
    pkg: &Package,
) {
    for sh in shapes {
        match sh {
            Shape::Placeholder(GraphicPlaceholder {
                chart_rel_id: Some(id),
                chart,
                ..
            }) => {
                *chart = rels
                    .get(id.as_str())
                    .and_then(|r| pkg.part_str(&resolve_part_path(part, &r.target)))
                    .map(|x| xml::chart::parse(&x));
            }
            Shape::Group(g) => resolve_charts(&mut g.children, rels, part, pkg),
            _ => {}
        }
    }
}
