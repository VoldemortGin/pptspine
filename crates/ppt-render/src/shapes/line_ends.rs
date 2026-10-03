//! 线端装饰(`a:headEnd` / `a:tailEnd`,ECMA-376 §20.1.8.38 / §20.1.8.57):在开放
//! 路径两端画 triangle / stealth / diamond / oval(实心,线色填充)与 arrow(开口,
//! 两段线描边)。
//!
//! **尺寸规则**(ECMA-376 只给 `sm`/`med`/`lg` 档名、不给数值,这里复刻 LibreOffice
//! `oox/source/drawingml/lineproperties.cxx::lclPushMarkerProperties` 的实际比例):
//! 基准宽 `base = max(线宽, 0.7 mm)`;箭头宽 = `w` 档系数 × `base`、长 = `len` 档系数
//! × `base`,系数 `sm/med/lg = 2/3/5`(开口 `arrow` 为 `2.5/3.5/5.5`)。
//!
//! **形状**(同取自 LibreOffice 标记多边形):triangle 尖在端点、底在 `len` 处;stealth
//! 在 triangle 上加 `0.6·len` 处的燕尾凹点;diamond / oval 以端点为中心(长 × 宽);
//! arrow 是尖在端点的两段折线。
//!
//! **线身缩短**:triangle 缩到箭头底(`len`)、stealth 缩到凹点(`0.6·len`),线身不再
//! 从实心箭头里穿出;diamond / oval 端点在形状中心、arrow 是线描,均不缩短。缩短量
//! 不超过端段弦长(单段线两端合计不超过弦长,按比例分摊)。

use ppt_core::model::{LineEnd, LineEndKind, LineEndSize};

use pdf_typeset::{Fill, Op, PathSeg, Stroke};

/// LibreOffice 的最小基准线宽:70 × 1/100 mm = 0.7 mm(pt)。
pub(super) const MIN_BASE_PT: f64 = 0.7 / 25.4 * 72.0;
/// stealth 燕尾凹点在箭头长度上的位置(LibreOffice 标记多边形 `(50, 60)`)。
const STEALTH_NOTCH: f64 = 0.6;
/// 四段三次 Bézier 近似椭圆的控制点系数。
const KAPPA: f64 = 0.552_284_749_830_793_4;

/// 线端装饰画不出来的原因(调用方据此发降级告警)。
#[derive(Debug, Clone, PartialEq)]
pub(super) enum EndSkip {
    /// 规范外的 `@type` 取值。
    UnknownKind(String),
}

/// 一端在路径上的位置与外向单位切线(从路径内部指向端点)。
#[derive(Debug, Clone, Copy, PartialEq)]
struct EndPoint {
    p: (f64, f64),
    dir: (f64, f64),
}

/// 是否是需要绘制的线端(种类非 `none`)。
pub(super) fn is_live(end: Option<&LineEnd>) -> bool {
    end.is_some_and(|e| e.kind != LineEndKind::None)
}

/// 档位系数(LibreOffice:实心 2/3/5,开口箭头 2.5/3.5/5.5)。
fn size_factor(size: LineEndSize, open: bool) -> f64 {
    let f = match size {
        LineEndSize::Small => 2.0,
        LineEndSize::Medium => 3.0,
        LineEndSize::Large => 5.0,
    };
    if open {
        f + 0.5
    } else {
        f
    }
}

/// 箭头 `(宽, 长)`(pt)。`base` 已取 `max(线宽, 最小基准)`。
fn end_dims(end: &LineEnd, base: f64) -> (f64, f64) {
    let open = end.kind == LineEndKind::Arrow;
    (
        size_factor(end.width, open) * base,
        size_factor(end.length, open) * base,
    )
}

/// 线身在该端应缩短的距离(pt)。
fn retreat(end: &LineEnd, base: f64) -> f64 {
    let (_, len) = end_dims(end, base);
    match end.kind {
        LineEndKind::Triangle => len,
        LineEndKind::Stealth => STEALTH_NOTCH * len,
        _ => 0.0,
    }
}

fn sub(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 - b.0, a.1 - b.1)
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    let d = sub(a, b);
    d.0.hypot(d.1)
}

/// `from → to` 的单位向量;两点重合 → `None`。
fn unit(from: (f64, f64), to: (f64, f64)) -> Option<(f64, f64)> {
    let d = sub(to, from);
    let n = d.0.hypot(d.1);
    (n > 1e-9).then(|| (d.0 / n, d.1 / n))
}

/// 单个开放子路径(一个 `MoveTo` + 若干 `LineTo`/`CurveTo`,无 `Close`)的控制点序列;
/// 闭合 / 多子路径 / 空路径 → `None`(PowerPoint 对闭合轮廓不画线端)。
fn open_points(segs: &[PathSeg]) -> Option<Vec<(f64, f64)>> {
    let (first, rest) = segs.split_first()?;
    let PathSeg::MoveTo { x, y } = *first else {
        return None;
    };
    let mut pts = vec![(x, y)];
    for s in rest {
        match *s {
            PathSeg::LineTo { x, y } => pts.push((x, y)),
            PathSeg::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => pts.extend([(x1, y1), (x2, y2), (x, y)]),
            PathSeg::MoveTo { .. } | PathSeg::Close => return None,
        }
    }
    (pts.len() >= 2).then_some(pts)
}

/// 两端位置 + 外向切线(曲线端取控制点方向 = 端点切线);全部点重合 → `None`。
fn path_ends(pts: &[(f64, f64)]) -> Option<(EndPoint, EndPoint)> {
    let p0 = pts[0];
    let pn = pts[pts.len() - 1];
    let next = pts.iter().copied().find(|&q| dist(q, p0) > 1e-9)?;
    let prev = pts.iter().rev().copied().find(|&q| dist(q, pn) > 1e-9)?;
    Some((
        EndPoint {
            p: p0,
            dir: unit(next, p0)?,
        },
        EndPoint {
            p: pn,
            dir: unit(prev, pn)?,
        },
    ))
}

/// 段的终点。
fn seg_end(s: &PathSeg) -> Option<(f64, f64)> {
    match *s {
        PathSeg::MoveTo { x, y } | PathSeg::LineTo { x, y } | PathSeg::CurveTo { x, y, .. } => {
            Some((x, y))
        }
        PathSeg::Close => None,
    }
}

/// 起点沿外向切线的反方向内移 `d`(首段为曲线时首控制点同移,保持切线)。
fn retreat_head(segs: &mut [PathSeg], dir: (f64, f64), d: f64) {
    let (dx, dy) = (-dir.0 * d, -dir.1 * d);
    if let Some(PathSeg::MoveTo { x, y }) = segs.first_mut() {
        *x += dx;
        *y += dy;
    }
    if let Some(PathSeg::CurveTo { x1, y1, .. }) = segs.get_mut(1) {
        *x1 += dx;
        *y1 += dy;
    }
}

/// 终点内移 `d`(末段为曲线时末控制点同移)。
fn retreat_tail(segs: &mut [PathSeg], dir: (f64, f64), d: f64) {
    let (dx, dy) = (-dir.0 * d, -dir.1 * d);
    match segs.last_mut() {
        Some(PathSeg::LineTo { x, y }) => {
            *x += dx;
            *y += dy;
        }
        Some(PathSeg::CurveTo { x2, y2, x, y, .. }) => {
            *x2 += dx;
            *y2 += dy;
            *x += dx;
            *y += dy;
        }
        _ => {}
    }
}

/// 局部坐标(`along` 沿线向内,`across` 垂直)→ 页坐标。
fn at(e: EndPoint, along: f64, across: f64) -> (f64, f64) {
    let (ux, uy) = e.dir;
    // 法向 = 切线逆时针转 90°;对称形状不关心朝向。
    let (nx, ny) = (-uy, ux);
    (
        e.p.0 - ux * along + nx * across,
        e.p.1 - uy * along + ny * across,
    )
}

fn polygon(pts: &[(f64, f64)]) -> Vec<PathSeg> {
    let mut segs = Vec::with_capacity(pts.len() + 1);
    for (i, &(x, y)) in pts.iter().enumerate() {
        segs.push(if i == 0 {
            PathSeg::MoveTo { x, y }
        } else {
            PathSeg::LineTo { x, y }
        });
    }
    segs.push(PathSeg::Close);
    segs
}

/// 以端点为中心的椭圆(沿线半轴 `a`、垂直半轴 `b`),四段 Bézier。
fn oval(e: EndPoint, a: f64, b: f64) -> Vec<PathSeg> {
    let (ka, kb) = (a * KAPPA, b * KAPPA);
    let p = |al: f64, ac: f64| at(e, al, ac);
    let curve = |c1: (f64, f64), c2: (f64, f64), to: (f64, f64)| PathSeg::CurveTo {
        x1: c1.0,
        y1: c1.1,
        x2: c2.0,
        y2: c2.1,
        x: to.0,
        y: to.1,
    };
    let start = p(a, 0.0);
    vec![
        PathSeg::MoveTo {
            x: start.0,
            y: start.1,
        },
        curve(p(a, kb), p(ka, b), p(0.0, b)),
        curve(p(-ka, b), p(-a, kb), p(-a, 0.0)),
        curve(p(-a, -kb), p(-ka, -b), p(0.0, -b)),
        curve(p(ka, -b), p(a, -kb), p(a, 0.0)),
        PathSeg::Close,
    ]
}

/// 一端的装饰 op;规范外种类 → `Err`。
fn end_op(end: &LineEnd, e: EndPoint, base: f64, line: &Stroke) -> Result<Option<Op>, EndSkip> {
    let (w, len) = end_dims(end, base);
    let half = w / 2.0;
    let fill = || {
        Some(Fill {
            color: line.color,
            alpha: line.alpha,
            even_odd: false,
        })
    };
    let closed = |segs: Vec<PathSeg>| Op::Path {
        segs,
        fill: fill(),
        stroke: None,
    };
    Ok(Some(match &end.kind {
        LineEndKind::None => return Ok(None),
        LineEndKind::Triangle => closed(polygon(&[
            at(e, 0.0, 0.0),
            at(e, len, half),
            at(e, len, -half),
        ])),
        LineEndKind::Stealth => closed(polygon(&[
            at(e, 0.0, 0.0),
            at(e, len, half),
            at(e, STEALTH_NOTCH * len, 0.0),
            at(e, len, -half),
        ])),
        LineEndKind::Diamond => closed(polygon(&[
            at(e, -len / 2.0, 0.0),
            at(e, 0.0, half),
            at(e, len / 2.0, 0.0),
            at(e, 0.0, -half),
        ])),
        LineEndKind::Oval => closed(oval(e, len / 2.0, half)),
        LineEndKind::Arrow => {
            let (a, t, b) = (at(e, len, half), at(e, 0.0, 0.0), at(e, len, -half));
            let mut stroke = line.clone();
            stroke.dashes.clear();
            Op::Path {
                segs: vec![
                    PathSeg::MoveTo { x: a.0, y: a.1 },
                    PathSeg::LineTo { x: t.0, y: t.1 },
                    PathSeg::LineTo { x: b.0, y: b.1 },
                ],
                fill: None,
                stroke: Some(stroke),
            }
        }
        LineEndKind::Other(name) => return Err(EndSkip::UnknownKind(name.clone())),
    }))
}

/// 给开放路径加线端装饰:按需缩短 `segs` 的线身,返回追加在线身之后绘制的装饰 op 与
/// 画不出的端(调用方发告警)。闭合 / 退化路径返回 `None`(不改 `segs`),由调用方
/// 判断是否需要告警(预设降级成包围盒时才算丢失)。
///
/// `min_base` 是已乘组合缩放的最小基准线宽(pt)。
pub(super) fn decorate(
    segs: &mut [PathSeg],
    head: Option<&LineEnd>,
    tail: Option<&LineEnd>,
    line: &Stroke,
    min_base: f64,
) -> Option<(Vec<Op>, Vec<EndSkip>)> {
    let pts = open_points(segs)?;
    let (h, t) = path_ends(&pts)?;
    let base = line.width.max(min_base);

    // 缩短量:不超过端段弦长;单段线两端合计不超过弦长(按比例分摊)。
    let mut rh = head.map_or(0.0, |e| retreat(e, base));
    let mut rt = tail.map_or(0.0, |e| retreat(e, base));
    let first_len = segs.get(1).and_then(seg_end).map_or(0.0, |q| dist(h.p, q));
    let last_start = segs
        .len()
        .checked_sub(2)
        .and_then(|i| segs.get(i))
        .and_then(seg_end);
    let last_len = last_start.map_or(0.0, |q| dist(q, t.p));
    if segs.len() == 2 {
        let total = rh + rt;
        if total > first_len && total > 0.0 {
            rh *= first_len / total;
            rt *= first_len / total;
        }
    } else {
        rh = rh.min(first_len);
        rt = rt.min(last_len);
    }

    let mut ops = Vec::new();
    let mut skipped = Vec::new();
    for (end, ep) in [(head, h), (tail, t)] {
        let Some(end) = end else { continue };
        match end_op(end, ep, base, line) {
            Ok(Some(op)) => ops.push(op),
            Ok(None) => {}
            Err(skip) => skipped.push(skip),
        }
    }
    // 画不出的端不缩短线身(保持原样到端点)。
    let drawn = |e: Option<&LineEnd>| e.is_some_and(|e| !matches!(e.kind, LineEndKind::Other(_)));
    if rh > 0.0 && drawn(head) {
        retreat_head(segs, h.dir, rh);
    }
    if rt > 0.0 && drawn(tail) {
        retreat_tail(segs, t.dir, rt);
    }
    Some((ops, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdf_typeset::Rgb;

    fn end(kind: LineEndKind, w: LineEndSize, len: LineEndSize) -> LineEnd {
        LineEnd {
            kind,
            width: w,
            length: len,
        }
    }

    fn med(kind: LineEndKind) -> LineEnd {
        end(kind, LineEndSize::Medium, LineEndSize::Medium)
    }

    fn hline(x0: f64, x1: f64) -> Vec<PathSeg> {
        vec![
            PathSeg::MoveTo { x: x0, y: 100.0 },
            PathSeg::LineTo { x: x1, y: 100.0 },
        ]
    }

    fn line_stroke(width: f64) -> Stroke {
        Stroke::new(
            Rgb {
                r: 1.0,
                g: 0.0,
                b: 0.0,
            },
            width,
        )
    }

    fn path_pts(op: &Op) -> Vec<(f64, f64)> {
        let Op::Path { segs, .. } = op else {
            panic!("expected path op");
        };
        segs.iter().filter_map(seg_end).collect()
    }

    fn approx(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    #[test]
    fn size_factors_follow_libreoffice() {
        // 4 pt 线(> 0.7 mm 最小基准):sm/med/lg = 2/3/5 倍线宽。
        let base = 4.0;
        let tri = |s| end_dims(&end(LineEndKind::Triangle, s, s), base);
        assert_eq!(tri(LineEndSize::Small), (8.0, 8.0));
        assert_eq!(tri(LineEndSize::Medium), (12.0, 12.0));
        assert_eq!(tri(LineEndSize::Large), (20.0, 20.0));
        // 开口 arrow 多 0.5 倍。
        let arr = end_dims(&med(LineEndKind::Arrow), base);
        assert_eq!(arr, (14.0, 14.0));
        // 宽 / 长两档独立。
        let mixed = end_dims(
            &end(
                LineEndKind::Triangle,
                LineEndSize::Large,
                LineEndSize::Small,
            ),
            base,
        );
        assert_eq!(mixed, (20.0, 8.0));
    }

    #[test]
    fn thin_lines_use_minimum_base() {
        let mut segs = hline(0.0, 200.0);
        let tail = med(LineEndKind::Triangle);
        let (ops, _) = decorate(
            &mut segs,
            None,
            Some(&tail),
            &line_stroke(0.75),
            MIN_BASE_PT,
        )
        .expect("open path");
        let pts = path_pts(&ops[0]);
        // 宽 = 3 × 0.7 mm ≈ 5.95 pt,而非 3 × 0.75。
        let w = (pts[1].1 - pts[2].1).abs();
        assert!((w - 3.0 * MIN_BASE_PT).abs() < 1e-6, "width {w}");
    }

    #[test]
    fn triangle_tail_tip_at_end_and_line_shortened_to_base() {
        let mut segs = hline(0.0, 200.0);
        let tail = med(LineEndKind::Triangle);
        let (ops, skipped) =
            decorate(&mut segs, None, Some(&tail), &line_stroke(2.0), 0.0).expect("open path");
        assert!(skipped.is_empty());
        assert_eq!(ops.len(), 1);
        let pts = path_pts(&ops[0]);
        assert!(approx(pts[0], (200.0, 100.0)), "tip {pts:?}");
        assert!(approx(pts[1], (194.0, 103.0)) || approx(pts[1], (194.0, 97.0)));
        // 线身终点缩到箭头底 x = 200 − 6。
        assert!(approx(seg_end(&segs[1]).unwrap(), (194.0, 100.0)));
        assert!(approx(seg_end(&segs[0]).unwrap(), (0.0, 100.0)));
        let Op::Path { fill, stroke, .. } = &ops[0] else {
            unreachable!()
        };
        assert!(stroke.is_none());
        assert_eq!(
            fill.as_ref().unwrap().color,
            Rgb {
                r: 1.0,
                g: 0.0,
                b: 0.0
            }
        );
    }

    #[test]
    fn head_points_backwards_and_shortens_start() {
        let mut segs = hline(10.0, 200.0);
        let head = med(LineEndKind::Stealth);
        let (ops, _) = decorate(&mut segs, Some(&head), None, &line_stroke(2.0), 0.0).unwrap();
        let pts = path_pts(&ops[0]);
        assert!(approx(pts[0], (10.0, 100.0)));
        assert!(approx(pts[2], (13.6, 100.0)), "notch {pts:?}");
        assert!((pts[1].0 - 16.0).abs() < 1e-6);
        // stealth 缩到凹点 0.6 × 6 = 3.6。
        assert!(approx(seg_end(&segs[0]).unwrap(), (13.6, 100.0)));
    }

    #[test]
    fn diamond_oval_arrow_centered_and_not_shortening() {
        for kind in [LineEndKind::Diamond, LineEndKind::Oval, LineEndKind::Arrow] {
            let mut segs = hline(0.0, 200.0);
            let tail = med(kind.clone());
            let (ops, _) = decorate(&mut segs, None, Some(&tail), &line_stroke(2.0), 0.0).unwrap();
            assert_eq!(ops.len(), 1, "{kind:?}");
            assert!(
                approx(seg_end(&segs[1]).unwrap(), (200.0, 100.0)),
                "{kind:?}"
            );
            let xs: Vec<f64> = path_pts(&ops[0]).iter().map(|p| p.0).collect();
            let max = xs.iter().copied().fold(f64::MIN, f64::max);
            match kind {
                // 中心在端点:沿线 ±len/2 = ±3。
                LineEndKind::Diamond | LineEndKind::Oval => assert!((max - 203.0).abs() < 1e-6),
                // 开口箭头尖在端点,两臂向内 len = 3.5 × 2 = 7。
                _ => {
                    assert!((max - 200.0).abs() < 1e-6);
                    let Op::Path { stroke, fill, .. } = &ops[0] else {
                        unreachable!()
                    };
                    assert!(fill.is_none());
                    assert!(stroke.as_ref().unwrap().dashes.is_empty());
                    let min = xs.iter().copied().fold(f64::MAX, f64::min);
                    assert!((min - 193.0).abs() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn diagonal_direction_follows_line() {
        let mut segs = vec![
            PathSeg::MoveTo { x: 0.0, y: 0.0 },
            PathSeg::LineTo { x: 30.0, y: 40.0 },
        ];
        let tail = med(LineEndKind::Triangle);
        decorate(&mut segs, None, Some(&tail), &line_stroke(2.0), 0.0).unwrap();
        // 沿 (0.6, 0.8) 方向退 6。
        assert!(approx(seg_end(&segs[1]).unwrap(), (26.4, 35.2)));
    }

    #[test]
    fn bent_connector_uses_last_segment_direction() {
        let mut segs = vec![
            PathSeg::MoveTo { x: 0.0, y: 0.0 },
            PathSeg::LineTo { x: 50.0, y: 0.0 },
            PathSeg::LineTo { x: 50.0, y: 80.0 },
        ];
        let (head, tail) = (med(LineEndKind::Triangle), med(LineEndKind::Triangle));
        let (ops, _) =
            decorate(&mut segs, Some(&head), Some(&tail), &line_stroke(2.0), 0.0).unwrap();
        assert_eq!(ops.len(), 2);
        assert!(approx(seg_end(&segs[0]).unwrap(), (6.0, 0.0)));
        assert!(approx(seg_end(&segs[1]).unwrap(), (50.0, 0.0)));
        assert!(approx(seg_end(&segs[2]).unwrap(), (50.0, 74.0)));
    }

    #[test]
    fn short_line_shortening_is_clamped() {
        let mut segs = hline(0.0, 8.0);
        let (head, tail) = (med(LineEndKind::Triangle), med(LineEndKind::Triangle));
        decorate(&mut segs, Some(&head), Some(&tail), &line_stroke(2.0), 0.0).unwrap();
        // 6 + 6 > 8 → 各缩 4,线身退化成一点但不反向。
        assert!(approx(seg_end(&segs[0]).unwrap(), (4.0, 100.0)));
        assert!(approx(seg_end(&segs[1]).unwrap(), (4.0, 100.0)));
    }

    #[test]
    fn curve_end_uses_control_point_tangent() {
        let mut segs = vec![
            PathSeg::MoveTo { x: 0.0, y: 0.0 },
            PathSeg::CurveTo {
                x1: 0.0,
                y1: 50.0,
                x2: 50.0,
                y2: 100.0,
                x: 100.0,
                y: 100.0,
            },
        ];
        let (head, tail) = (med(LineEndKind::Triangle), med(LineEndKind::Triangle));
        let (ops, _) =
            decorate(&mut segs, Some(&head), Some(&tail), &line_stroke(2.0), 0.0).unwrap();
        // 头端切线竖直向上(指向 (0,0) 外侧 = −y),尾端水平向右。
        let head_pts = path_pts(&ops[0]);
        assert!((head_pts[1].1 - 6.0).abs() < 1e-6, "{head_pts:?}");
        let tail_pts = path_pts(&ops[1]);
        assert!((tail_pts[1].0 - 94.0).abs() < 1e-6, "{tail_pts:?}");
        assert!(approx(seg_end(&segs[1]).unwrap(), (94.0, 100.0)));
    }

    #[test]
    fn closed_path_gets_no_ends() {
        let mut segs = hline(0.0, 50.0);
        segs.push(PathSeg::Close);
        let tail = med(LineEndKind::Triangle);
        assert!(decorate(&mut segs, None, Some(&tail), &line_stroke(1.0), 0.0).is_none());
    }

    #[test]
    fn unknown_kind_is_skipped_without_shortening() {
        let mut segs = hline(0.0, 200.0);
        let tail = med(LineEndKind::Other("bogus".into()));
        let (ops, skipped) =
            decorate(&mut segs, None, Some(&tail), &line_stroke(2.0), 0.0).unwrap();
        assert!(ops.is_empty());
        assert_eq!(skipped, vec![EndSkip::UnknownKind("bogus".into())]);
        assert!(approx(seg_end(&segs[1]).unwrap(), (200.0, 100.0)));
    }

    #[test]
    fn none_kind_draws_nothing() {
        let mut segs = hline(0.0, 200.0);
        let tail = med(LineEndKind::None);
        let (ops, skipped) =
            decorate(&mut segs, None, Some(&tail), &line_stroke(2.0), 0.0).unwrap();
        assert!(ops.is_empty() && skipped.is_empty());
        assert!(approx(seg_end(&segs[1]).unwrap(), (200.0, 100.0)));
    }
}
