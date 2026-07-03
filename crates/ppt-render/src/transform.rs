//! 组合仿射与形状级变换(B-4/B-5)。
//!
//! 坐标契约:一切矩阵都在**左上原点、y 向下**的页坐标(pt)里表达
//! (`pdf_typeset::ops` 的约定;发射时引擎统一 y 翻转)。
//!
//! - **组合(B-5)**:子坐标先按 `(child − chOff) · (ext/chExt) + off` 重映射,
//!   再绕组合矩形中心做翻转、旋转(OOXML 变换次序:flip 先、rot 后)。
//!   纯"平移 + 均匀正缩放"的组合走 [`Flatten`] 预乘(文本框保持与"拆组等价形状"
//!   逐字重合——B-5 绿条的孪生等价门);含旋转 / 翻转 / 非均匀缩放的组合退回
//!   `Op::Group { transform }`(引擎 `q cm … Q`,嵌套自然复合)。
//! - **形状级 rot/flip(B-4)**:绕形状矩形中心的仿射,包在 `Op::Group` 里。
//!   pptx `rot` 是顺时针 1/60000 度;`Matrix::rotate(+deg)` 在 y 向下坐标里
//!   恰是视觉顺时针,直接使用。(文本框旋转走引擎 `TextBoxSpec::rotation_deg`,
//!   其语义是视觉**逆**时针,换算取负。)

use ppt_core::geom::Rect as EmuRect;
use ppt_core::model::Xfrm;
use ppt_core::resolved::ResolvedGroup;

use pdf_typeset::{Matrix, Rect};

/// 均匀缩放判定容差(浮点安全余量;组合缩放来自 EMU 整数比,足够宽松)。
const UNIFORM_EPS: f64 = 1e-6;

/// 累计的"平移 + 均匀正缩放"仿射(子坐标 → 页坐标):`p' = p·s + (dx, dy)`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Flatten {
    pub s: f64,
    pub dx: f64,
    pub dy: f64,
}

impl Flatten {
    /// 恒等仿射。
    pub const IDENTITY: Flatten = Flatten {
        s: 1.0,
        dx: 0.0,
        dy: 0.0,
    };

    /// 先应用 `inner`(子空间 → 本空间),再应用 `self`(本空间 → 页)。
    pub fn after(self, inner: Flatten) -> Flatten {
        Flatten {
            s: self.s * inner.s,
            dx: inner.dx * self.s + self.dx,
            dy: inner.dy * self.s + self.dy,
        }
    }

    /// EMU 矩形 → 已映射的页坐标矩形(pt)。
    pub fn map_emu_rect(self, r: EmuRect) -> Rect {
        let (x, y, w, h) = r.to_points();
        let x0 = x * self.s + self.dx;
        let y0 = y * self.s + self.dy;
        Rect::new(x0, y0, x0 + w * self.s, y0 + h * self.s)
    }

    /// 折成引擎矩阵(退回 `Op::Group` 路径时用)。
    pub fn to_matrix(self) -> Matrix {
        Matrix::new(self.s, 0.0, 0.0, self.s, self.dx, self.dy)
    }
}

/// 一个组合自身的变换(子坐标 → 父坐标)。
pub(crate) enum GroupTransform {
    /// 平移 + 均匀正缩放:可预乘进 [`Flatten`](含恒等)。
    Flat(Flatten),
    /// 含旋转 / 翻转 / 非均匀缩放:退回 `Op::Group { transform }`。
    Full(Matrix),
}

/// 组合的子坐标重映射 + 自身旋转/翻转(B-5,§3.e)。
///
/// `rect`/`child_rect` 齐备才做缩放重映射(`chExt` 为 0 的轴退化为纯平移);
/// 缺失任一侧时子坐标视作已在父空间(恒等重映射),但 `rot`/flip 仍绕
/// `rect` 中心生效(无 `rect` 则无中心可言,一并跳过)。
pub(crate) fn group_transform(g: &ResolvedGroup) -> GroupTransform {
    let remap = match (g.rect, g.child_rect) {
        (Some(rect), Some(child)) => {
            let (ox, oy, ow, oh) = rect.to_points();
            let (cx, cy, cw, ch) = child.to_points();
            let sx = if cw.abs() > f64::EPSILON {
                ow / cw
            } else {
                1.0
            };
            let sy = if ch.abs() > f64::EPSILON {
                oh / ch
            } else {
                1.0
            };
            (sx, sy, ox - cx * sx, oy - cy * sy)
        }
        _ => (1.0, 1.0, 0.0, 0.0),
    };
    let (sx, sy, dx, dy) = remap;

    if g.xfrm.is_identity() {
        if (sx - sy).abs() <= UNIFORM_EPS && sx > 0.0 {
            return GroupTransform::Flat(Flatten { s: sx, dx, dy });
        }
        return GroupTransform::Full(Matrix::new(sx, 0.0, 0.0, sy, dx, dy));
    }

    // 旋转/翻转绕组合矩形中心(父空间)生效;次序:缩放重映射 → 翻转 → 旋转。
    let remap_m = Matrix::new(sx, 0.0, 0.0, sy, dx, dy);
    let m = match g.rect {
        Some(rect) => {
            let (ox, oy, ow, oh) = rect.to_points();
            let center = (ox + ow / 2.0, oy + oh / 2.0);
            Matrix::concat(&remap_m, &spin_about(g.xfrm, center))
        }
        None => remap_m,
    };
    GroupTransform::Full(m)
}

/// 形状级 rot/flip(B-4,§3.d)→ 绕 `rect`(pt,页坐标)中心的仿射;恒等 → `None`。
pub(crate) fn shape_transform(xfrm: Xfrm, rect: Rect) -> Option<Matrix> {
    if xfrm.is_identity() {
        return None;
    }
    let center = ((rect.x0 + rect.x1) / 2.0, (rect.y0 + rect.y1) / 2.0);
    Some(spin_about(xfrm, center))
}

/// 绕 `center` 的翻转 + 旋转(flip 先、rot 后;`rot` 顺时针为正,y 向下坐标里
/// `Matrix::rotate(+deg)` 即视觉顺时针)。
fn spin_about(xfrm: Xfrm, center: (f64, f64)) -> Matrix {
    let (cx, cy) = center;
    let mut m = Matrix::translate(-cx, -cy);
    if xfrm.flip_h || xfrm.flip_v {
        let fx = if xfrm.flip_h { -1.0 } else { 1.0 };
        let fy = if xfrm.flip_v { -1.0 } else { 1.0 };
        m = Matrix::concat(&m, &Matrix::scale(fx, fy));
    }
    if xfrm.rot != 0 {
        m = Matrix::concat(&m, &Matrix::rotate(xfrm.rot_deg()));
    }
    Matrix::concat(&m, &Matrix::translate(cx, cy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdf_typeset::Point;
    use ppt_core::geom::Rect as EmuRect;
    use ppt_core::resolved::ResolvedShape;

    /// 12700 EMU = 1 pt,便于用小整数写测试矩形。
    const PT: i64 = 12_700;

    fn group(
        rect: Option<EmuRect>,
        child_rect: Option<EmuRect>,
        rot: i32,
        flip_h: bool,
        flip_v: bool,
    ) -> ResolvedGroup {
        ResolvedGroup {
            rect,
            child_rect,
            xfrm: Xfrm {
                rot,
                flip_h,
                flip_v,
            },
            children: Vec::new(),
        }
    }

    fn assert_pt(m: &Matrix, p: (f64, f64), want: (f64, f64)) {
        let got = m.transform_point(Point::new(p.0, p.1));
        assert!(
            (got.x - want.0).abs() < 1e-9 && (got.y - want.1).abs() < 1e-9,
            "point {p:?} mapped to ({}, {}), want {want:?}",
            got.x,
            got.y
        );
    }

    /// 平移 + 均匀缩放:`(child − chOff)·(ext/chExt) + off` 逐点核对。
    #[test]
    fn flat_translate_scale_remap() {
        // child (10,20,100,50) -> parent (30,40,200,100):s=2,平移随 chOff 走。
        let g = group(
            Some(EmuRect::new(30 * PT, 40 * PT, 200 * PT, 100 * PT)),
            Some(EmuRect::new(10 * PT, 20 * PT, 100 * PT, 50 * PT)),
            0,
            false,
            false,
        );
        let GroupTransform::Flat(f) = group_transform(&g) else {
            panic!("expected flat transform");
        };
        assert!((f.s - 2.0).abs() < 1e-12);
        // chOff (10,20) 映射到 off (30,40)。
        assert!((10.0 * f.s + f.dx - 30.0).abs() < 1e-9);
        assert!((20.0 * f.s + f.dy - 40.0).abs() < 1e-9);
        // 子空间右下角映射到父矩形右下角。
        assert!((110.0 * f.s + f.dx - 230.0).abs() < 1e-9);
        assert!((70.0 * f.s + f.dy - 140.0).abs() < 1e-9);
    }

    /// 非均匀缩放退回矩阵路径,映射仍精确。
    #[test]
    fn non_uniform_scale_falls_back_to_matrix() {
        let g = group(
            Some(EmuRect::new(0, 0, 200 * PT, 50 * PT)),
            Some(EmuRect::new(0, 0, 100 * PT, 100 * PT)),
            0,
            false,
            false,
        );
        let GroupTransform::Full(m) = group_transform(&g) else {
            panic!("expected full matrix");
        };
        assert_pt(&m, (100.0, 100.0), (200.0, 50.0));
        assert_pt(&m, (50.0, 0.0), (100.0, 0.0));
    }

    /// 旋转 90°(顺时针)+ 重映射的组合:角点落位核对。
    #[test]
    fn rotation_composes_after_remap() {
        // 恒等重映射 + rot=90°,矩形 (0,0,100,100),中心 (50,50)。
        let g = group(
            Some(EmuRect::new(0, 0, 100 * PT, 100 * PT)),
            Some(EmuRect::new(0, 0, 100 * PT, 100 * PT)),
            90 * 60_000,
            false,
            false,
        );
        let GroupTransform::Full(m) = group_transform(&g) else {
            panic!("expected full matrix");
        };
        // y 向下坐标里顺时针 90°:左上 (0,0) → 右上 (100,0)。
        assert_pt(&m, (0.0, 0.0), (100.0, 0.0));
        assert_pt(&m, (100.0, 0.0), (100.0, 100.0));
        assert_pt(&m, (50.0, 50.0), (50.0, 50.0));
    }

    /// 翻转先于旋转(OOXML 次序):flipH + rot=90° 下角点落位。
    #[test]
    fn flip_applies_before_rotation() {
        let g = group(
            Some(EmuRect::new(0, 0, 100 * PT, 100 * PT)),
            Some(EmuRect::new(0, 0, 100 * PT, 100 * PT)),
            90 * 60_000,
            true,
            false,
        );
        let GroupTransform::Full(m) = group_transform(&g) else {
            panic!("expected full matrix");
        };
        // (0,0) --flipH 绕中心--> (100,0) --顺时针 90°--> (100,100)。
        assert_pt(&m, (0.0, 0.0), (100.0, 100.0));
        // 中心不动。
        assert_pt(&m, (50.0, 50.0), (50.0, 50.0));
    }

    /// 平移 + 缩放 + 旋转 + 翻转的全组合:核对一个非平凡点。
    #[test]
    fn full_combo_translate_scale_rotate_flip() {
        // child (0,0,100,100) -> rect (100,0,200,200)(s=2)+ flipV + rot 180°。
        let g = group(
            Some(EmuRect::new(100 * PT, 0, 200 * PT, 200 * PT)),
            Some(EmuRect::new(0, 0, 100 * PT, 100 * PT)),
            180 * 60_000,
            false,
            true,
        );
        let GroupTransform::Full(m) = group_transform(&g) else {
            panic!("expected full matrix");
        };
        // 重映射:(0,0)->(100,0);flipV 绕中心 (200,100):(100,0)->(100,200);
        // rot180 绕中心:(100,200)->(300,0)。
        assert_pt(&m, (0.0, 0.0), (300.0, 0.0));
        // 子空间中心始终映射到父矩形中心。
        assert_pt(&m, (50.0, 50.0), (200.0, 100.0));
    }

    /// 两层嵌套 Flat 组合:`after` 复合与逐层映射一致。
    #[test]
    fn nested_flat_composition() {
        let outer = Flatten {
            s: 2.0,
            dx: 10.0,
            dy: 20.0,
        };
        let inner = Flatten {
            s: 0.5,
            dx: 4.0,
            dy: 6.0,
        };
        let both = outer.after(inner);
        // 逐层:p=(8,8) --inner--> (8,10) --outer--> (26,40)。
        let (x, y) = (8.0 * both.s + both.dx, 8.0 * both.s + both.dy);
        assert!((x - 26.0).abs() < 1e-12 && (y - 40.0).abs() < 1e-12);
    }

    /// 两层嵌套、外层 Flat + 内层旋转:嵌套 `Op::Group` 的矩阵按
    /// `concat(inner, outer)` 复合(引擎 q cm 嵌套语义)。
    #[test]
    fn nested_full_composes_with_outer_flatten() {
        let outer = Flatten {
            s: 2.0,
            dx: 100.0,
            dy: 0.0,
        };
        let g = group(
            Some(EmuRect::new(0, 0, 100 * PT, 100 * PT)),
            Some(EmuRect::new(0, 0, 100 * PT, 100 * PT)),
            90 * 60_000,
            false,
            false,
        );
        let GroupTransform::Full(inner) = group_transform(&g) else {
            panic!("expected full matrix");
        };
        let total = Matrix::concat(&inner, &outer.to_matrix());
        // (0,0) --rot90 绕 (50,50)--> (100,0) --outer--> (300,0)。
        assert_pt(&total, (0.0, 0.0), (300.0, 0.0));
    }

    /// 形状级变换:恒等 → None;flipH → 绕中心镜像。
    #[test]
    fn shape_transform_identity_and_flip() {
        let rect = Rect::new(0.0, 0.0, 100.0, 50.0);
        assert!(shape_transform(Xfrm::default(), rect).is_none());
        let m = shape_transform(
            Xfrm {
                rot: 0,
                flip_h: true,
                flip_v: false,
            },
            rect,
        )
        .expect("flip transform");
        assert_pt(&m, (0.0, 0.0), (100.0, 0.0));
        assert_pt(&m, (100.0, 50.0), (0.0, 50.0));
    }

    /// 45° 旋转的形状级矩阵:中心不动,角点旋到对角线上。
    #[test]
    fn shape_transform_rotation_45() {
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let m = shape_transform(
            Xfrm {
                rot: 45 * 60_000,
                flip_h: false,
                flip_v: false,
            },
            rect,
        )
        .expect("rotation");
        assert_pt(&m, (50.0, 50.0), (50.0, 50.0));
        // 顶边中点 (50,0) 顺时针 45° 后落在右上对角方向。
        let p = m.transform_point(Point::new(50.0, 0.0));
        assert!(
            p.x > 50.0 && p.y < 50.0,
            "clockwise in y-down coords: {p:?}"
        );
        let r = ((p.x - 50.0).powi(2) + (p.y - 50.0).powi(2)).sqrt();
        assert!((r - 50.0).abs() < 1e-9, "radius preserved");
    }

    /// group_transform 对缺 rect/child_rect 的组合:恒等 Flat。
    #[test]
    fn missing_rects_yield_identity() {
        let g = group(None, None, 0, false, false);
        let GroupTransform::Flat(f) = group_transform(&g) else {
            panic!("expected flat");
        };
        assert_eq!(f, Flatten::IDENTITY);
        // 只为编译器:children 字段被用到。
        assert!(matches!(
            g.children.first(),
            None | Some(ResolvedShape::Placeholder(_))
        ));
    }
}
