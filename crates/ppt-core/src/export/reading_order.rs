//! 视觉阅读顺序:把形状树**展平**(组合拆开、子矩形映射到 slide 绝对坐标)后,按包围盒排序。
//!
//! 算法(确定性、无递归,绝不 panic)是递归 XY-cut(Nagy & Seth 1984,版面分析的经典做法)
//! 的迭代实现。对一组形状:
//! 1. 分别沿纵轴(top..bottom)与横轴(left..right)找**空白切口**:把区间按起点扫描,
//!    与已扫区间重叠不超过容差的位置都是候选切口,切口宽 = 空白间距(可为不超过容差的负值);
//! 2. 取两轴中**最宽**的一个切口一刀两分(并列时优先横切 = 先上后下),两半各自回到第 1 步;
//!    横切 → 上半先出,竖切 → 左半先出。于是标题(上方横向空白)先出,其下两栏之间的竖向
//!    空白(栏间距)宽于栏内块与块的间隙 → "左栏整列先于右栏";
//! 3. 两轴都无切口(形状两向交叠):按 top 分行(与行首 top 相差 ≤ `tol_y` 视为同一行),
//!    行内按 left。
//!
//! docling / MarkItDown 只做第 3 步(top→left 排序),遇到两栏且行不对齐的版面会左右交错;
//! 反过来,栏间距小于行距的网格 / 键值对版面在这里按列读(XY-cut 的固有取舍)。
//!
//! **行阈值 `tol_y` = slide 高度的 2%**(4:3 / 16:9 标准高 7.5" 时 ≈ 0.15" ≈ 10.8 pt):
//! - 只吸收手工摆放 / 对齐抖动(PowerPoint 智能参考线对齐后的残差远小于此),不随内容变化;
//! - 不取"形状高度中位数的一半":它依赖本页形状构成——满页图片会把阈值撑大到把上下两行并成
//!   一行,一堆小图标又让阈值趋零;固定比例更稳定、可预期。
//! - slide 高度未知(`sldSz` 缺失 = 0)时才回退到"形状高度中位数的一半"。
//!
//! 横向容差 `tol_x` 同理取 slide 宽度的 2%。没有矩形的形状排在最后(保持文档相对顺序)。
//!
//! **复杂度保护**:每次切分对当前组排序扫描一遍,切分不均衡时(一列纵向文本框、完全重叠的连接线
//! 每次只切下一个元素)总量是 O(n² log n)。所以累计切分步数(进入切分的组大小之和)设预算
//! `4·n·⌈log₂(n+1)⌉ + 20 000`:超出后剩余各组退回第 3 步的按 (top, left) 行排序(稳定、确定),
//! 最坏情形降为 O(n log² n)。常数项让 200 个形状以内的任何版式都走完整 XY-cut——正常幻灯片的
//! 结果与引入预算前逐个一致(见测试里固化的语料摘要)。

use crate::geom::{Emu, Rect};
use crate::model::Shape;
use crate::resolved::ResolvedShape;

/// 展平后的一个叶子形状:原始形状 + (若可配对)继承链解析后的同一形状 + 绝对矩形。
#[derive(Debug, Clone, Copy)]
pub struct FlatShape<'a> {
    pub shape: &'a Shape,
    /// 继承链解析后的同一形状(占位符几何 / 项目符号已物化);无解析信息为 `None`。
    pub resolved: Option<&'a ResolvedShape>,
    /// slide 绝对坐标下的包围盒(组合仿射已应用;旋转忽略,按未旋转外框计)。
    pub rect: Option<Rect>,
}

/// 把形状树展平成叶子序列(文档顺序;组合的子形状就地展开)。`resolved` 是同一棵树的
/// 解析结果(见 [`pair_resolved`]);矩形优先取解析后的物化矩形(占位符继承几何)。
pub fn flatten<'a>(
    shapes: &'a [Shape],
    resolved: Option<&'a [ResolvedShape]>,
) -> Vec<FlatShape<'a>> {
    let mut out = Vec::new();
    // 显式栈代替递归(组合嵌套深度已由解析层限到 64,这里仍不吃调用栈)。
    let mut stack: Vec<Frame<'a>> =
        vec![(shapes, pair_resolved(shapes, resolved), Affine::IDENTITY, 0)];
    while let Some((list, res, map, i)) = stack.pop() {
        let Some(sh) = list.get(i) else {
            continue;
        };
        stack.push((list, res, map, i + 1));
        let rs = res.and_then(|r| r.get(i));
        if let Shape::Group(g) = sh {
            let inner = map.then_group(g.rect, g.child_rect);
            let kids = match rs {
                Some(ResolvedShape::Group(rg)) => Some(rg.children.as_slice()),
                _ => None,
            };
            stack.push((&g.children, pair_resolved(&g.children, kids), inner, 0));
        } else {
            let rect = rs.and_then(resolved_rect).or_else(|| raw_rect(sh));
            out.push(FlatShape {
                shape: sh,
                resolved: rs,
                rect: rect.map(|r| map.apply(r)),
            });
        }
    }
    out
}

/// 展平栈帧:(同级形状, 配对的解析形状, 子 → slide 仿射, 下一个下标)。
type Frame<'a> = (&'a [Shape], Option<&'a [ResolvedShape]>, Affine, usize);

/// 原始形状与解析形状逐个配对:长度相同且逐个种类一致 → 配对;解析侧更长时(如前置了
/// 母版 / 版式形状)尝试按尾部对齐;都不成立 → `None`(调用方退回原始矩形 / 直接格式)。
pub fn pair_resolved<'a>(
    raw: &[Shape],
    resolved: Option<&'a [ResolvedShape]>,
) -> Option<&'a [ResolvedShape]> {
    let res = resolved?;
    let tail = res.get(res.len().checked_sub(raw.len())?..)?;
    raw.iter()
        .zip(tail)
        .all(|(a, b)| same_kind(a, b))
        .then_some(tail)
}

fn same_kind(a: &Shape, b: &ResolvedShape) -> bool {
    matches!(
        (a, b),
        (Shape::TextBox(_), ResolvedShape::TextBox(_))
            | (Shape::Table(_), ResolvedShape::Table(_))
            | (Shape::Picture(_), ResolvedShape::Picture(_))
            | (Shape::Group(_), ResolvedShape::Group(_))
            | (Shape::Auto(_), ResolvedShape::Auto(_))
            | (Shape::Connector(_), ResolvedShape::Connector(_))
            | (Shape::Placeholder(_), ResolvedShape::Placeholder(_))
    )
}

fn raw_rect(sh: &Shape) -> Option<Rect> {
    match sh {
        Shape::TextBox(t) => t.rect,
        Shape::Table(t) => t.rect,
        Shape::Picture(p) => p.rect,
        Shape::Group(g) => g.rect,
        Shape::Auto(a) => a.rect,
        Shape::Connector(c) => c.rect,
        Shape::Placeholder(p) => p.rect,
    }
}

fn resolved_rect(sh: &ResolvedShape) -> Option<Rect> {
    match sh {
        ResolvedShape::TextBox(t) => t.rect,
        ResolvedShape::Auto(a) => a.rect,
        ResolvedShape::Connector(c) => c.rect,
        ResolvedShape::Table(t) => t.rect,
        ResolvedShape::Picture(p) => p.rect,
        ResolvedShape::Group(g) => g.rect,
        ResolvedShape::Placeholder(p) => p.rect,
    }
}

/// 轴对齐仿射(子坐标 → slide 坐标):`x' = x·sx + tx`,`y' = y·sy + ty`。
#[derive(Debug, Clone, Copy)]
struct Affine {
    sx: f64,
    sy: f64,
    tx: f64,
    ty: f64,
}

impl Affine {
    const IDENTITY: Affine = Affine {
        sx: 1.0,
        sy: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// 叠加一个组合的子空间重映射 `(p − chOff)·(ext/chExt) + off`;任一侧缺失 / `chExt`
    /// 为 0 的轴退化为恒等(与渲染侧 B-5 语义一致)。
    fn then_group(self, rect: Option<Rect>, child: Option<Rect>) -> Affine {
        let (Some(r), Some(c)) = (rect, child) else {
            return self;
        };
        let axis = |off: Emu, ext: Emu, ch_off: Emu, ch_ext: Emu| -> (f64, f64) {
            if ch_ext == 0 {
                (1.0, 0.0)
            } else {
                let k = ext as f64 / ch_ext as f64;
                (k, off as f64 - ch_off as f64 * k)
            }
        };
        let (kx, bx) = axis(r.x, r.w, c.x, c.w);
        let (ky, by) = axis(r.y, r.h, c.y, c.h);
        Affine {
            sx: self.sx * kx,
            sy: self.sy * ky,
            tx: self.sx * bx + self.tx,
            ty: self.sy * by + self.ty,
        }
    }

    fn apply(self, r: Rect) -> Rect {
        let x0 = r.x as f64 * self.sx + self.tx;
        let y0 = r.y as f64 * self.sy + self.ty;
        let x1 = (r.x as f64 + r.w as f64) * self.sx + self.tx;
        let y1 = (r.y as f64 + r.h as f64) * self.sy + self.ty;
        let (x0, x1) = (x0.min(x1), x0.max(x1));
        let (y0, y1) = (y0.min(y1), y0.max(y1));
        let emu = |v: f64| v.round().clamp(i64::MIN as f64, i64::MAX as f64) as Emu;
        Rect::new(emu(x0), emu(y0), emu(x1 - x0), emu(y1 - y0))
    }
}

/// 视觉阅读顺序:返回 `rects` 的下标排列(见模块文档的算法与阈值依据)。
/// `slide_size` 为画布 `(cx, cy)`(EMU);`None` 矩形排在最后、保持相对顺序。
pub fn reading_order(rects: &[Option<Rect>], slide_size: (Emu, Emu)) -> Vec<usize> {
    order_counted(rects, slide_size).0
}

/// [`reading_order`] 的实现,另返回切分步数(进入切分的组大小之和,= 排序 / 扫描的元素量)。
fn order_counted(rects: &[Option<Rect>], slide_size: (Emu, Emu)) -> (Vec<usize>, usize) {
    let mut steps = 0usize;
    let boxes: Vec<(usize, Box2)> = rects
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.map(|r| (i, Box2::from(r))))
        .collect();
    let tol_y = tolerance(slide_size.1, boxes.iter().map(|(_, b)| b.y1 - b.y0));
    let tol_x = tolerance(slide_size.0, boxes.iter().map(|(_, b)| b.x1 - b.x0));
    let budget = work_budget(boxes.len());

    let mut out = Vec::with_capacity(rects.len());
    // 显式工作栈(后处理的一半先压栈),绝不递归。
    let mut work: Vec<Vec<(usize, Box2)>> = vec![boxes];
    while let Some(group) = work.pop() {
        if group.len() <= 1 {
            out.extend(group.iter().map(|(i, _)| *i));
            continue;
        }
        if steps > budget {
            // 切分步数超出预算:剩余组退回按 (top, left) 行排序(见模块文档)。
            out.extend(rows_then_left(group, tol_y));
            continue;
        }
        steps += group.len();
        let rows = best_cut(&group, tol_y, |b| (b.y0, b.y1));
        let cols = best_cut(&group, tol_x, |b| (b.x0, b.x1));
        let cut = match (rows, cols) {
            (Some(r), Some(c)) => Some(if c.0 > r.0 { c } else { r }),
            (r, c) => r.or(c),
        };
        match cut {
            Some((_, first, second)) => {
                work.push(second);
                work.push(first);
            }
            None => out.extend(rows_then_left(group, tol_y)),
        }
    }
    out.extend(
        rects
            .iter()
            .enumerate()
            .filter(|(_, r)| r.is_none())
            .map(|(i, _)| i),
    );
    (out, steps)
}

/// 切分步数预算:`4·n·⌈log₂(n+1)⌉ + 20 000`(见模块文档)。
fn work_budget(n: usize) -> usize {
    let lg = (usize::BITS - n.leading_zeros()) as usize;
    n.saturating_mul(lg)
        .saturating_mul(4)
        .saturating_add(20_000)
}

/// 一组待排序的形状:(原下标, 包围盒)。
type Part = Vec<(usize, Box2)>;

/// 包围盒(f64,便于比较;来源是 EMU 整数)。
#[derive(Debug, Clone, Copy)]
struct Box2 {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl From<Rect> for Box2 {
    fn from(r: Rect) -> Self {
        let (w, h) = (r.w.max(0) as f64, r.h.max(0) as f64);
        Box2 {
            x0: r.x as f64,
            y0: r.y as f64,
            x1: r.x as f64 + w,
            y1: r.y as f64 + h,
        }
    }
}

/// 容差:画布该轴尺寸的 2%;尺寸未知时取形状该轴尺寸中位数的一半。
fn tolerance(canvas: Emu, extents: impl Iterator<Item = f64>) -> f64 {
    if canvas > 0 {
        return canvas as f64 * 0.02;
    }
    let mut v: Vec<f64> = extents.collect();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2] / 2.0
}

/// 沿一个轴找最宽的空白切口:返回 `(切口宽, 前半, 后半)`;重叠超过 `tol` 的位置不可切。
/// 并列取靠前的切口;两半内部保持文档顺序(确定性)。
fn best_cut(
    group: &[(usize, Box2)],
    tol: f64,
    axis: impl Fn(&Box2) -> (f64, f64),
) -> Option<(f64, Part, Part)> {
    let mut sorted = group.to_vec();
    sorted.sort_by(|a, b| axis(&a.1).0.total_cmp(&axis(&b.1).0).then(a.0.cmp(&b.0)));
    let mut end = f64::NEG_INFINITY;
    let mut best: Option<(f64, usize)> = None;
    for (k, item) in sorted.iter().enumerate() {
        let (s, e) = axis(&item.1);
        if k > 0 {
            let gap = s - end;
            if gap >= -tol && best.is_none_or(|(g, _)| gap > g) {
                best = Some((gap, k));
            }
        }
        end = end.max(e);
    }
    let (gap, k) = best?;
    let mut second = sorted.split_off(k);
    let mut first = sorted;
    first.sort_by_key(|(i, _)| *i);
    second.sort_by_key(|(i, _)| *i);
    Some((gap, first, second))
}

/// 两向都切不开时:按 top 分行(与行首 top 相差 ≤ `tol_y` 同行),行内按 left。
fn rows_then_left(mut group: Vec<(usize, Box2)>, tol_y: f64) -> Vec<usize> {
    group.sort_by(|a, b| a.1.y0.total_cmp(&b.1.y0).then(a.0.cmp(&b.0)));
    let mut out = Vec::with_capacity(group.len());
    let mut row: Vec<(usize, Box2)> = Vec::new();
    let mut row_top = f64::NEG_INFINITY;
    let flush = |row: &mut Vec<(usize, Box2)>, out: &mut Vec<usize>| {
        row.sort_by(|a, b| a.1.x0.total_cmp(&b.1.x0).then(a.0.cmp(&b.0)));
        out.extend(row.drain(..).map(|(i, _)| i));
    };
    for item in group {
        if !row.is_empty() && item.1.y0 - row_top > tol_y {
            flush(&mut row, &mut out);
        }
        if row.is_empty() {
            row_top = item.1.y0;
        }
        row.push(item);
    }
    flush(&mut row, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GroupShape, TextFrame};

    const SLIDE: (Emu, Emu) = (9_144_000, 6_858_000);

    fn r(x: Emu, y: Emu, w: Emu, h: Emu) -> Option<Rect> {
        Some(Rect::new(x, y, w, h))
    }

    /// 确定性伪随机(xorshift64*),生成"正常幻灯片"版式语料。
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn range(&mut self, n: u64) -> i64 {
            (self.next() % n.max(1)) as i64
        }
    }

    /// 一页"正常"版式(≤ 120 个形状):随机散布 / 网格 / 两栏 / 标题 + 列表 / 互相重叠 / 缺矩形。
    fn normal_layout(rng: &mut Rng) -> Vec<Option<Rect>> {
        let n = 1 + rng.range(120) as usize;
        let style = rng.range(6);
        (0..n)
            .map(|i| {
                let i = i as i64;
                match style {
                    0 => r(
                        rng.range(8_500_000),
                        rng.range(6_400_000),
                        200_000 + rng.range(3_000_000),
                        100_000 + rng.range(1_500_000),
                    ),
                    1 => r(
                        (i % 6) * 1_500_000 + rng.range(50_000),
                        (i / 6) * 500_000 + rng.range(50_000),
                        1_400_000,
                        450_000,
                    ),
                    2 => r(
                        if i % 2 == 0 { 500_000 } else { 4_800_000 },
                        1_200_000 + (i / 2) * 400_000 + rng.range(150_000),
                        3_500_000,
                        350_000,
                    ),
                    3 if i == 0 => r(500_000, 300_000, 8_000_000, 900_000),
                    3 => r(
                        700_000 + rng.range(3) * 300_000,
                        1_300_000 + i * 120_000,
                        7_000_000,
                        110_000,
                    ),
                    4 => r(
                        rng.range(2_000_000),
                        rng.range(2_000_000),
                        3_000_000 + rng.range(4_000_000),
                        2_000_000 + rng.range(3_000_000),
                    ),
                    _ if rng.range(10) == 0 => None,
                    _ => r(
                        rng.range(9_000_000),
                        rng.range(6_800_000),
                        rng.range(900_000),
                        rng.range(600_000),
                    ),
                }
            })
            .collect()
    }

    /// FNV-1a 摘要:把整份语料的排序结果压成一个数。
    fn corpus_digest(order: impl Fn(&[Option<Rect>]) -> Vec<usize>) -> u64 {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for _ in 0..2_000 {
            let rects = normal_layout(&mut rng);
            for i in order(&rects) {
                for b in (i as u32).to_le_bytes() {
                    h ^= u64::from(b);
                    h = h.wrapping_mul(0x0100_0000_01b3);
                }
            }
            h ^= 0xff;
        }
        h
    }

    /// 正常幻灯片的阅读顺序被固化:2 000 页确定性语料(≤ 120 个形状 / 页,含散布、网格、两栏、
    /// 列表、重叠、缺矩形)的排序结果摘要必须与改造前一致(摘要取自改造前的实现)。
    #[test]
    fn normal_layout_corpus_order_is_frozen() {
        assert_eq!(corpus_digest(|r| reading_order(r, SLIDE)), FROZEN_DIGEST);
    }

    const FROZEN_DIGEST: u64 = 13_309_350_722_119_740_429;

    /// 切分步数的 n·log n 上界(含常数项;见 `work_budget`)。
    fn n_log_n(n: usize) -> usize {
        let lg = usize::BITS - n.leading_zeros();
        8 * n * lg as usize + 40_000
    }

    /// 退化输入(一列纵向文本框、完全重叠的连接线、斜对角)下切分步数与 n log n 同阶,而不是 n²。
    #[test]
    fn degenerate_inputs_take_n_log_n_steps() {
        for n in [2_000_usize, 8_000, 16_000] {
            let column: Vec<_> = (0..n).map(|i| r(0, i as i64 * 300, 100, 100)).collect();
            let stacked: Vec<_> = (0..n).map(|_| r(0, 0, 1, 1)).collect();
            let diagonal: Vec<_> = (0..n)
                .map(|i| r(i as i64 * 300, i as i64 * 300, 100, 100))
                .collect();
            for (name, rects) in [
                ("column", column),
                ("stacked", stacked),
                ("diagonal", diagonal),
            ] {
                let (order, steps) = order_counted(&rects, SLIDE);
                assert_eq!(order.len(), n);
                assert!(steps <= n_log_n(n), "{name} n={n}: {steps} 步");
                if name != "stacked" {
                    // 退回 (y, x) 排序后,纵列 / 斜对角仍是自上而下。
                    assert!(order.iter().copied().eq(0..n), "{name}");
                } else {
                    assert!(order.iter().copied().eq(0..n), "完全重叠保持文档顺序");
                }
            }
        }
    }

    #[test]
    fn two_columns_read_column_by_column() {
        // 标题在最上;左栏 3 块、右栏 2 块,行互不对齐(右栏下沉半行)。
        let rects = vec![
            r(5_000_000, 2_000_000, 3_500_000, 800_000), // 0 右 1
            r(500_000, 1_500_000, 3_500_000, 800_000),   // 1 左 1
            r(500_000, 2_600_000, 3_500_000, 800_000),   // 2 左 2
            r(5_000_000, 3_100_000, 3_500_000, 800_000), // 3 右 2
            r(500_000, 3_700_000, 3_500_000, 800_000),   // 4 左 3
            r(500_000, 300_000, 8_000_000, 900_000),     // 5 标题
        ];
        assert_eq!(reading_order(&rects, SLIDE), vec![5, 1, 2, 4, 0, 3]);
    }

    #[test]
    fn staggered_columns_with_tiny_cross_gaps_still_read_by_column() {
        // 左右块交错、跨栏只重叠 / 间隔不足容差:栏间距(70 万 EMU)是最宽切口。
        let rects = vec![
            r(4_800_000, 2_100_000, 3_500_000, 700_000), // 0 R1
            r(600_000, 1_800_000, 3_500_000, 700_000),   // 1 L1
            r(4_800_000, 3_300_000, 3_500_000, 700_000), // 2 R2
            r(600_000, 2_700_000, 3_500_000, 700_000),   // 3 L2
            r(600_000, 3_600_000, 3_500_000, 700_000),   // 4 L3
            r(838_200, 365_125, 7_772_400, 1_325_563),   // 5 标题
        ];
        assert_eq!(reading_order(&rects, SLIDE), vec![5, 1, 3, 4, 0, 2]);
    }

    #[test]
    fn aligned_rows_read_left_to_right() {
        let rects = vec![
            r(5_000_000, 1_500_000, 3_000_000, 500_000),
            r(500_000, 1_520_000, 3_000_000, 500_000), // 同一行(top 抖动 20000 EMU)
            r(500_000, 300_000, 8_000_000, 900_000),
        ];
        assert_eq!(reading_order(&rects, SLIDE), vec![2, 1, 0]);
    }

    #[test]
    fn overlapping_shapes_fall_back_to_top_then_left() {
        // 两向交叠:大底图 + 叠在其上的两个标签。
        let rects = vec![
            r(0, 0, 9_000_000, 6_000_000),
            r(4_000_000, 1_000_000, 1_000_000, 500_000),
            r(1_000_000, 1_050_000, 1_000_000, 500_000),
        ];
        assert_eq!(reading_order(&rects, SLIDE), vec![0, 2, 1]);
    }

    #[test]
    fn rectless_shapes_go_last_in_document_order() {
        let rects = vec![None, r(0, 100, 10, 10), None, r(0, 0, 10, 10)];
        assert_eq!(reading_order(&rects, SLIDE), vec![3, 1, 0, 2]);
    }

    #[test]
    fn group_children_map_to_absolute_coordinates() {
        let child = |x, y| {
            Shape::TextBox(TextFrame {
                rect: Some(Rect::new(x, y, 100, 100)),
                ..TextFrame::default()
            })
        };
        // 组合把子空间 (0,0)-(1000,1000) 缩放 2 倍放到 (5000, 7000)。
        let shapes = vec![Shape::Group(GroupShape {
            rect: Some(Rect::new(5_000, 7_000, 2_000, 2_000)),
            child_rect: Some(Rect::new(0, 0, 1_000, 1_000)),
            children: vec![child(0, 0), child(500, 500)],
            ..GroupShape::default()
        })];
        let flat = flatten(&shapes, None);
        assert_eq!(flat.len(), 2);
        assert_eq!(flat[0].rect, Some(Rect::new(5_000, 7_000, 200, 200)));
        assert_eq!(flat[1].rect, Some(Rect::new(6_000, 8_000, 200, 200)));
    }
}
