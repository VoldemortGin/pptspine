//! `a:custGeom` 求值:参考线公式 -> 路径命令 -> 引擎 [`PathSeg`]。
//!
//! 公式求值器是纯函数([`eval_fmla`] / [`eval_guides`]),仓内无现成实现(预设几何由
//! `pdf-typeset::preset` 内部硬编码,不对外暴露公式引擎)。数值一律 `f64`;任何未定义名字、
//! 除零、NaN / 无穷都返回 `Err`,调用方([`build_paths`])整体降级为 `None`(渲染退回包围盒
//! 近似 + 告警),绝不 panic。

use std::collections::HashMap;

use pdf_typeset::{PathSeg, Rect};
use ppt_core::custgeom::{CustGeom, PathCmd, PathFill};

/// 求值失败(未定义名字 / 参数个数不对 / 除零 / 非有限数 / 不认识的运算符)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EvalError;

/// 参考线环境:名字 -> 值。
pub(crate) type Env = HashMap<String, f64>;

/// 一条已建好的路径(引擎坐标,单位 pt)。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BuiltPath {
    pub segs: Vec<PathSeg>,
    /// 该路径是否参与填充(`fill != none`)。
    pub fill: bool,
    /// 该路径是否描边(`stroke` 属性)。
    pub stroke: bool,
}

/// 内置变量表(形状宽高 `w` × `h`,EMU):外框坐标、等分、`ss` / `ls`、角度常量。
pub(crate) fn builtins(w: f64, h: f64) -> Env {
    let (ss, ls) = (w.min(h), w.max(h));
    let mut env = Env::new();
    let mut put = |k: &str, v: f64| {
        env.insert(k.to_string(), v);
    };
    put("w", w);
    put("h", h);
    put("l", 0.0);
    put("t", 0.0);
    put("r", w);
    put("b", h);
    put("hc", w / 2.0);
    put("vc", h / 2.0);
    put("ss", ss);
    put("ls", ls);
    for n in [2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 32.0] {
        put(&format!("wd{n}"), w / n);
        put(&format!("hd{n}"), h / n);
    }
    for n in [2.0, 4.0, 6.0, 8.0, 16.0, 32.0] {
        put(&format!("ssd{n}"), ss / n);
    }
    // 角度常量(1/60000 度):cd2 = 180°。
    put("cd2", 10_800_000.0);
    put("cd4", 5_400_000.0);
    put("cd8", 2_700_000.0);
    put("3cd4", 16_200_000.0);
    put("3cd8", 8_100_000.0);
    put("5cd8", 13_500_000.0);
    put("7cd8", 18_900_000.0);
    env
}

/// 1/60000 度 -> 弧度。
fn ang_to_rad(a: f64) -> f64 {
    a / 60_000.0 * std::f64::consts::PI / 180.0
}

/// 弧度 -> 1/60000 度。
fn rad_to_ang(r: f64) -> f64 {
    r * 180.0 / std::f64::consts::PI * 60_000.0
}

/// 一个操作数:数字字面量或参考线 / 内置变量名。`inf` / `nan` 这类词不当数字。
fn operand(tok: &str, env: &Env) -> Result<f64, EvalError> {
    let numeric = tok
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.'));
    let v = match tok.parse::<f64>() {
        Ok(v) if numeric => v,
        _ => *env.get(tok).ok_or(EvalError)?,
    };
    finite(v)
}

fn finite(v: f64) -> Result<f64, EvalError> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(EvalError)
    }
}

/// 求一条公式 `"op arg arg arg"`(规范 §20.1.9.11 的全部运算符)。参数个数必须恰好匹配。
pub(crate) fn eval_fmla(fmla: &str, env: &Env) -> Result<f64, EvalError> {
    let mut it = fmla.split_whitespace();
    let op = it.next().ok_or(EvalError)?;
    let args: Vec<&str> = it.collect();
    let argc = match op {
        "val" | "abs" | "sqrt" => 1,
        "at2" | "cos" | "sin" | "tan" | "max" | "min" => 2,
        "*/" | "+-" | "+/" | "?:" | "cat2" | "sat2" | "mod" | "pin" => 3,
        _ => return Err(EvalError),
    };
    if args.len() != argc {
        return Err(EvalError);
    }
    let a = |i: usize| operand(args[i], env);
    let v = match op {
        "val" => a(0)?,
        "*/" => {
            let z = a(2)?;
            if z == 0.0 {
                return Err(EvalError);
            }
            a(0)? * a(1)? / z
        }
        "+-" => a(0)? + a(1)? - a(2)?,
        "+/" => {
            let z = a(2)?;
            if z == 0.0 {
                return Err(EvalError);
            }
            (a(0)? + a(1)?) / z
        }
        "?:" => {
            if a(0)? > 0.0 {
                a(1)?
            } else {
                a(2)?
            }
        }
        "abs" => a(0)?.abs(),
        "at2" => rad_to_ang(a(1)?.atan2(a(0)?)),
        "cat2" => a(0)? * a(2)?.atan2(a(1)?).cos(),
        "sat2" => a(0)? * a(2)?.atan2(a(1)?).sin(),
        "cos" => a(0)? * ang_to_rad(a(1)?).cos(),
        "sin" => a(0)? * ang_to_rad(a(1)?).sin(),
        "tan" => a(0)? * ang_to_rad(a(1)?).tan(),
        "max" => a(0)?.max(a(1)?),
        "min" => a(0)?.min(a(1)?),
        "mod" => {
            let (x, y, z) = (a(0)?, a(1)?, a(2)?);
            (x * x + y * y + z * z).sqrt()
        }
        "pin" => {
            let (x, y, z) = (a(0)?, a(1)?, a(2)?);
            if y < x {
                x
            } else if y > z {
                z
            } else {
                y
            }
        }
        "sqrt" => a(0)?.sqrt(),
        _ => return Err(EvalError),
    };
    finite(v)
}

/// 按声明顺序求 `avLst` 再 `gdLst` 的全部参考线(后者可引用前者;同名后者遮蔽前者 / 内置,
/// 前向引用与自引用在求值时尚未定义,按未定义名字报错)。
pub(crate) fn eval_guides(g: &CustGeom, w: f64, h: f64) -> Result<Env, EvalError> {
    let mut env = builtins(w, h);
    for gd in g.av_lst.iter().chain(&g.gd_lst) {
        let v = eval_fmla(&gd.fmla, &env)?;
        env.insert(gd.name.clone(), v);
    }
    Ok(env)
}

/// 视觉角(射线与 x 轴夹角)-> 椭圆参数角,保持 2π 的整数倍连续(扫过角可跨圈)。
fn param_angle(theta: f64, w_r: f64, h_r: f64) -> f64 {
    let (s, c) = theta.sin_cos();
    theta - s.atan2(c) + (w_r * s).atan2(h_r * c)
}

/// 路径空间里的一条已求值子路径构建器。
struct PathBuilder {
    segs: Vec<PathSeg>,
    cur: Option<(f64, f64)>,
    start: Option<(f64, f64)>,
}

impl PathBuilder {
    fn move_to(&mut self, p: (f64, f64)) {
        self.segs.push(PathSeg::MoveTo { x: p.0, y: p.1 });
        self.cur = Some(p);
        self.start = Some(p);
    }

    fn line_to(&mut self, p: (f64, f64)) {
        if self.cur.is_none() {
            return self.move_to(p);
        }
        self.segs.push(PathSeg::LineTo { x: p.0, y: p.1 });
        self.cur = Some(p);
    }

    fn cubic_to(&mut self, c1: (f64, f64), c2: (f64, f64), p: (f64, f64)) {
        if self.cur.is_none() {
            self.move_to(c1);
        }
        self.segs.push(PathSeg::CurveTo {
            x1: c1.0,
            y1: c1.1,
            x2: c2.0,
            y2: c2.1,
            x: p.0,
            y: p.1,
        });
        self.cur = Some(p);
    }

    fn quad_to(&mut self, q: (f64, f64), p: (f64, f64)) {
        let p0 = self.cur.unwrap_or(q);
        let c1 = (
            p0.0 + 2.0 / 3.0 * (q.0 - p0.0),
            p0.1 + 2.0 / 3.0 * (q.1 - p0.1),
        );
        let c2 = (p.0 + 2.0 / 3.0 * (q.0 - p.0), p.1 + 2.0 / 3.0 * (q.1 - p.1));
        self.cubic_to(c1, c2, p);
    }

    fn close(&mut self) {
        if self.cur.is_some() {
            self.segs.push(PathSeg::Close);
            self.cur = self.start;
        }
    }

    /// 从当前点出发的椭圆弧(角度已是弧度),每段 ≤ 90° 的三次贝塞尔。无当前点 / 半径非正 /
    /// 扫过角为零 → 忽略。
    fn arc_to(&mut self, w_r: f64, h_r: f64, st: f64, sw: f64) {
        let Some(cur) = self.cur else {
            return;
        };
        if w_r <= 0.0 || h_r <= 0.0 || sw == 0.0 {
            return;
        }
        // 扫过角最多两整圈,防止对抗输入产生海量分段。
        let sw = sw.clamp(-4.0 * std::f64::consts::PI, 4.0 * std::f64::consts::PI);
        let t1 = param_angle(st, w_r, h_r);
        let t2 = param_angle(st + sw, w_r, h_r);
        let dt = t2 - t1;
        let (cx, cy) = (cur.0 - w_r * t1.cos(), cur.1 - h_r * t1.sin());
        let n = (dt.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
        let step = dt / n as f64;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let at = |t: f64| (cx + w_r * t.cos(), cy + h_r * t.sin());
        for i in 0..n {
            let (a0, a1) = (t1 + step * i as f64, t1 + step * (i + 1) as f64);
            let (p0, p1) = (at(a0), at(a1));
            // 参数曲线切向 (−a sin t, b cos t)。
            let c1 = (p0.0 - k * w_r * a0.sin(), p0.1 + k * h_r * a0.cos());
            let c2 = (p1.0 + k * w_r * a1.sin(), p1.1 - k * h_r * a1.cos());
            self.cubic_to(c1, c2, p1);
        }
    }
}

/// 构建全部路径。`w_emu` / `h_emu` 是形状自身尺寸(EMU,内置变量 `w` / `h` 与缺省路径坐标系),
/// `frame` 是页坐标下的外框(组合缩放后)。失败(求值错误 / 非有限坐标)返回 `None`。
pub(crate) fn build_paths(
    g: &CustGeom,
    w_emu: f64,
    h_emu: f64,
    frame: Rect,
) -> Option<Vec<BuiltPath>> {
    let env = eval_guides(g, w_emu, h_emu).ok()?;
    let mut out = Vec::with_capacity(g.paths.len());
    for path in &g.paths {
        let dim = |v: Option<i64>, own: f64| v.filter(|&v| v > 0).map_or(own, |v| v as f64);
        let (pw, ph) = (dim(path.w, w_emu), dim(path.h, h_emu));
        let sx = if pw > 0.0 {
            (frame.x1 - frame.x0) / pw
        } else {
            0.0
        };
        let sy = if ph > 0.0 {
            (frame.y1 - frame.y0) / ph
        } else {
            0.0
        };
        let num = |s: &str| operand(s, &env);
        let pt = |p: &ppt_core::custgeom::CustPt| -> Result<(f64, f64), EvalError> {
            Ok((num(&p.x)?, num(&p.y)?))
        };
        let mut b = PathBuilder {
            segs: Vec::new(),
            cur: None,
            start: None,
        };
        for cmd in &path.cmds {
            let r: Result<(), EvalError> = (|| {
                match cmd {
                    PathCmd::MoveTo(p) => b.move_to(pt(p)?),
                    PathCmd::LnTo(p) => b.line_to(pt(p)?),
                    PathCmd::CubicBezTo(c1, c2, p) => b.cubic_to(pt(c1)?, pt(c2)?, pt(p)?),
                    PathCmd::QuadBezTo(q, p) => b.quad_to(pt(q)?, pt(p)?),
                    PathCmd::ArcTo {
                        w_r,
                        h_r,
                        st_ang,
                        sw_ang,
                    } => b.arc_to(
                        num(w_r)?,
                        num(h_r)?,
                        ang_to_rad(num(st_ang)?),
                        ang_to_rad(num(sw_ang)?),
                    ),
                    PathCmd::Close => b.close(),
                }
                Ok(())
            })();
            r.ok()?;
        }
        let segs = b
            .segs
            .into_iter()
            .map(|s| map_seg(s, frame, sx, sy))
            .collect::<Option<Vec<_>>>()?;
        out.push(BuiltPath {
            segs,
            fill: path.fill != PathFill::None,
            stroke: path.stroke,
        });
    }
    Some(out)
}

/// 路径空间 -> 页坐标;出现非有限值返回 `None`。
fn map_seg(s: PathSeg, f: Rect, sx: f64, sy: f64) -> Option<PathSeg> {
    let x = |v: f64| f.x0 + v * sx;
    let y = |v: f64| f.y0 + v * sy;
    let s = match s {
        PathSeg::MoveTo { x: a, y: b } => PathSeg::MoveTo { x: x(a), y: y(b) },
        PathSeg::LineTo { x: a, y: b } => PathSeg::LineTo { x: x(a), y: y(b) },
        PathSeg::CurveTo {
            x1,
            y1,
            x2,
            y2,
            x: a,
            y: b,
        } => PathSeg::CurveTo {
            x1: x(x1),
            y1: y(y1),
            x2: x(x2),
            y2: y(y2),
            x: x(a),
            y: y(b),
        },
        PathSeg::Close => PathSeg::Close,
    };
    let ok = match s {
        PathSeg::MoveTo { x, y } | PathSeg::LineTo { x, y } => x.is_finite() && y.is_finite(),
        PathSeg::CurveTo {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        } => [x1, y1, x2, y2, x, y].iter().all(|v| v.is_finite()),
        PathSeg::Close => true,
    };
    ok.then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppt_core::custgeom::{CustPath, CustPt, Guide};

    fn env() -> Env {
        builtins(1000.0, 600.0)
    }

    fn ev(f: &str) -> Result<f64, EvalError> {
        eval_fmla(f, &env())
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn operators() {
        assert_eq!(ev("val 42"), Ok(42.0));
        assert_eq!(ev("val -7"), Ok(-7.0));
        assert_eq!(ev("*/ 10 6 4"), Ok(15.0));
        assert_eq!(ev("+- 10 6 4"), Ok(12.0));
        assert_eq!(ev("+/ 10 6 4"), Ok(4.0));
        assert_eq!(ev("?: 1 5 9"), Ok(5.0));
        assert_eq!(ev("?: 0 5 9"), Ok(9.0));
        assert_eq!(ev("?: -3 5 9"), Ok(9.0));
        assert_eq!(ev("abs -12"), Ok(12.0));
        assert_eq!(ev("max 3 8"), Ok(8.0));
        assert_eq!(ev("min 3 8"), Ok(3.0));
        assert_eq!(ev("mod 2 3 6"), Ok(7.0));
        assert_eq!(ev("sqrt 49"), Ok(7.0));
        assert_eq!(ev("pin 0 -5 10"), Ok(0.0));
        assert_eq!(ev("pin 0 5 10"), Ok(5.0));
        assert_eq!(ev("pin 0 15 10"), Ok(10.0));
    }

    #[test]
    fn trig_uses_sixty_thousandths_of_a_degree() {
        // sin / cos / tan:`op x ang` = x * f(ang)
        assert!(near(ev("sin 100 5400000").unwrap(), 100.0)); // 90°
        assert!(near(ev("cos 100 0").unwrap(), 100.0));
        assert!(near(ev("cos 100 10800000").unwrap(), -100.0)); // 180°
        assert!(near(ev("tan 100 2700000").unwrap(), 100.0)); // 45°
                                                              // at2 x y = atan2(y, x),单位 1/60000 度
        assert!(near(ev("at2 1 1").unwrap(), 2_700_000.0));
        assert!(near(ev("at2 0 1").unwrap(), 5_400_000.0));
        assert!(near(ev("at2 -1 0").unwrap(), 10_800_000.0));
        // cat2 x y z = x*cos(at2 z y);sat2 x y z = x*sin(at2 z y)
        assert!(near(
            ev("cat2 100 1 1").unwrap(),
            100.0 * (45f64).to_radians().cos()
        ));
        assert!(near(
            ev("sat2 100 1 1").unwrap(),
            100.0 * (45f64).to_radians().sin()
        ));
    }

    #[test]
    fn builtin_variables_and_angle_constants() {
        let cases: &[(&str, f64)] = &[
            ("w", 1000.0),
            ("h", 600.0),
            ("l", 0.0),
            ("t", 0.0),
            ("r", 1000.0),
            ("b", 600.0),
            ("hc", 500.0),
            ("vc", 300.0),
            ("wd2", 500.0),
            ("wd4", 250.0),
            ("wd8", 125.0),
            ("wd10", 100.0),
            ("hd2", 300.0),
            ("hd3", 200.0),
            ("hd6", 100.0),
            ("ss", 600.0),
            ("ls", 1000.0),
            ("ssd2", 300.0),
            ("ssd4", 150.0),
            ("ssd8", 75.0),
            ("ssd16", 37.5),
            ("ssd32", 18.75),
            ("cd2", 10_800_000.0),
            ("cd4", 5_400_000.0),
            ("cd8", 2_700_000.0),
            ("3cd4", 16_200_000.0),
            ("3cd8", 8_100_000.0),
            ("5cd8", 13_500_000.0),
            ("7cd8", 18_900_000.0),
        ];
        for (name, want) in cases {
            assert_eq!(ev(&format!("val {name}")), Ok(*want), "{name}");
        }
        assert_eq!(ev("*/ w 1 2"), Ok(500.0));
    }

    #[test]
    fn bad_formulas_are_errors_not_panics() {
        assert_eq!(ev("*/ 1 2 0"), Err(EvalError)); // 除零
        assert_eq!(ev("+/ 1 2 0"), Err(EvalError));
        assert_eq!(ev("val nosuch"), Err(EvalError)); // 未定义引用
        assert_eq!(ev("sqrt -1"), Err(EvalError)); // NaN
        assert_eq!(ev("*/ 1e308 1e308 1"), Err(EvalError)); // 无穷
        assert_eq!(ev("+- 1 2"), Err(EvalError)); // 参数不足
        assert_eq!(ev("bogus 1 2 3"), Err(EvalError)); // 未知运算符
        assert_eq!(ev(""), Err(EvalError));
        assert_eq!(ev("val"), Err(EvalError));
        assert_eq!(ev("val 1 2"), Err(EvalError)); // 多余参数
    }

    fn guide(name: &str, fmla: &str) -> Guide {
        Guide {
            name: name.into(),
            fmla: fmla.into(),
        }
    }

    #[test]
    fn guides_evaluate_in_declaration_order_with_shadowing() {
        let g = CustGeom {
            av_lst: vec![guide("adj", "val 25000")],
            gd_lst: vec![
                guide("x1", "*/ w adj 100000"),
                // 同名后声明者遮蔽内置 `hc`;其后的引用取新值
                guide("hc", "val 7"),
                guide("x2", "+- hc 1 0"),
            ],
            paths: vec![],
        };
        let env = eval_guides(&g, 1000.0, 600.0).unwrap();
        assert_eq!(env["x1"], 250.0);
        assert_eq!(env["x2"], 8.0);
    }

    #[test]
    fn forward_and_self_references_are_undefined() {
        let fwd = CustGeom {
            gd_lst: vec![guide("a", "val bb"), guide("bb", "val 1")],
            ..CustGeom::default()
        };
        assert!(eval_guides(&fwd, 10.0, 10.0).is_err());
        let selfref = CustGeom {
            gd_lst: vec![guide("a", "+- a 1 0")],
            ..CustGeom::default()
        };
        assert!(eval_guides(&selfref, 10.0, 10.0).is_err());
    }

    fn pt(x: &str, y: &str) -> CustPt {
        CustPt {
            x: x.into(),
            y: y.into(),
        }
    }

    fn path(w: Option<i64>, h: Option<i64>, cmds: Vec<PathCmd>) -> CustPath {
        CustPath {
            w,
            h,
            fill: PathFill::Norm,
            stroke: true,
            extrusion_ok: false,
            cmds,
        }
    }

    fn geom(paths: Vec<CustPath>) -> CustGeom {
        CustGeom {
            paths,
            ..CustGeom::default()
        }
    }

    fn frame() -> Rect {
        Rect::new(100.0, 200.0, 400.0, 400.0) // 300 x 200 pt
    }

    #[test]
    fn triangle_scales_from_path_space_to_frame() {
        let g = geom(vec![path(
            Some(10),
            Some(10),
            vec![
                PathCmd::MoveTo(pt("0", "10")),
                PathCmd::LnTo(pt("10", "10")),
                PathCmd::LnTo(pt("5", "0")),
                PathCmd::Close,
            ],
        )]);
        let built = build_paths(&g, 3_810_000.0, 2_540_000.0, frame()).unwrap();
        assert_eq!(built.len(), 1);
        assert_eq!(
            built[0].segs,
            vec![
                PathSeg::MoveTo { x: 100.0, y: 400.0 },
                PathSeg::LineTo { x: 400.0, y: 400.0 },
                PathSeg::LineTo { x: 250.0, y: 200.0 },
                PathSeg::Close,
            ]
        );
        assert!(built[0].fill && built[0].stroke);
    }

    #[test]
    fn missing_path_size_uses_shape_size_and_guide_names_resolve() {
        let g = CustGeom {
            gd_lst: vec![guide("mx", "*/ w 1 2")],
            paths: vec![path(
                None,
                None,
                vec![PathCmd::MoveTo(pt("l", "t")), PathCmd::LnTo(pt("mx", "b"))],
            )],
            ..CustGeom::default()
        };
        let built = build_paths(&g, 3_810_000.0, 2_540_000.0, frame()).unwrap();
        assert_eq!(
            built[0].segs,
            vec![
                PathSeg::MoveTo { x: 100.0, y: 200.0 },
                PathSeg::LineTo { x: 250.0, y: 400.0 },
            ]
        );
    }

    #[test]
    fn cubic_and_quad_beziers() {
        let g = geom(vec![path(
            Some(100),
            Some(100),
            vec![
                PathCmd::MoveTo(pt("0", "0")),
                PathCmd::CubicBezTo(pt("0", "50"), pt("50", "100"), pt("100", "100")),
                PathCmd::QuadBezTo(pt("100", "0"), pt("0", "0")),
            ],
        )]);
        let built = build_paths(&g, 100.0, 100.0, Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        let segs = &built[0].segs;
        assert_eq!(
            segs[1],
            PathSeg::CurveTo {
                x1: 0.0,
                y1: 50.0,
                x2: 50.0,
                y2: 100.0,
                x: 100.0,
                y: 100.0
            }
        );
        // 二次 -> 三次:c1 = p0 + 2/3 (q - p0),c2 = p2 + 2/3 (q - p2)
        let PathSeg::CurveTo {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        } = segs[2]
        else {
            panic!("quad must become a cubic");
        };
        assert!(near(x1, 100.0 + 2.0 / 3.0 * 0.0) && near(y1, 100.0 + 2.0 / 3.0 * -100.0));
        assert!(near(x2, 2.0 / 3.0 * 100.0) && near(y2, 2.0 / 3.0 * 0.0));
        assert!(near(x, 0.0) && near(y, 0.0));
    }

    #[test]
    fn arc_to_becomes_cubics_of_at_most_90_degrees_ending_on_the_ellipse() {
        // 当前点 (100,50) 在椭圆(中心 (50,50),半径 50x50)的 0° 处,扫 180° 到 (0,50)。
        let g = geom(vec![path(
            Some(100),
            Some(100),
            vec![
                PathCmd::MoveTo(pt("100", "50")),
                PathCmd::ArcTo {
                    w_r: "50".into(),
                    h_r: "50".into(),
                    st_ang: "0".into(),
                    sw_ang: "10800000".into(),
                },
            ],
        )]);
        let built = build_paths(&g, 100.0, 100.0, Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        let segs = &built[0].segs;
        assert_eq!(segs.len(), 3, "move + 2 cubics (180° / 90°)");
        let PathSeg::CurveTo { x, y, .. } = segs[2] else {
            panic!("arc must be cubic");
        };
        assert!(near(x, 0.0) && near(y, 50.0), "ends at ({x},{y})");
        let PathSeg::CurveTo { x: mx, y: my, .. } = segs[1] else {
            panic!("arc must be cubic");
        };
        // 0° -> 90°(y 向下)中点在 (50, 100)
        assert!(near(mx, 50.0) && near(my, 100.0), "mid ({mx},{my})");
    }

    #[test]
    fn negative_sweep_and_full_circle() {
        let g = geom(vec![path(
            Some(100),
            Some(100),
            vec![
                PathCmd::MoveTo(pt("50", "0")),
                PathCmd::ArcTo {
                    w_r: "50".into(),
                    h_r: "50".into(),
                    st_ang: "16200000".into(), // 270° = 顶点
                    sw_ang: "21600000".into(),
                },
                PathCmd::Close,
            ],
        )]);
        let built = build_paths(&g, 100.0, 100.0, Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        // move + 4 cubics + close
        assert_eq!(built[0].segs.len(), 6);
        let PathSeg::CurveTo { x, y, .. } = built[0].segs[4] else {
            panic!()
        };
        assert!(near(x, 50.0) && near(y, 0.0));

        let neg = geom(vec![path(
            Some(100),
            Some(100),
            vec![
                PathCmd::MoveTo(pt("100", "50")),
                PathCmd::ArcTo {
                    w_r: "50".into(),
                    h_r: "50".into(),
                    st_ang: "0".into(),
                    sw_ang: "-5400000".into(),
                },
            ],
        )]);
        let b = build_paths(&neg, 100.0, 100.0, Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        let PathSeg::CurveTo { x, y, .. } = b[0].segs[1] else {
            panic!()
        };
        assert!(near(x, 50.0) && near(y, 0.0), "counter-clockwise to top");
    }

    #[test]
    fn elliptical_arc_uses_visual_angles() {
        // wR=100, hR=50:视觉角 45° 的点是 (cx+a, cy+b) 方向,即参数角 atan(2)... 终点必在椭圆上。
        let g = geom(vec![path(
            Some(200),
            Some(100),
            vec![
                PathCmd::MoveTo(pt("200", "50")),
                PathCmd::ArcTo {
                    w_r: "100".into(),
                    h_r: "50".into(),
                    st_ang: "0".into(),
                    sw_ang: "2700000".into(), // 视觉 45°
                },
            ],
        )]);
        let b = build_paths(&g, 200.0, 100.0, Rect::new(0.0, 0.0, 200.0, 100.0)).unwrap();
        let PathSeg::CurveTo { x, y, .. } = b[0].segs[1] else {
            panic!()
        };
        let (dx, dy) = (x - 100.0, y - 50.0);
        assert!(near((dx / 100.0).powi(2) + (dy / 50.0).powi(2), 1.0));
        assert!(near(dy / dx, 1.0), "visual 45° ray, got slope {}", dy / dx);
    }

    #[test]
    fn multiple_paths_and_fill_flags() {
        let mut none = path(
            Some(10),
            Some(10),
            vec![PathCmd::MoveTo(pt("0", "0")), PathCmd::LnTo(pt("10", "10"))],
        );
        none.fill = PathFill::None;
        let mut nostroke = none.clone();
        nostroke.fill = PathFill::Darken;
        nostroke.stroke = false;
        let g = geom(vec![none, nostroke]);
        let b = build_paths(&g, 10.0, 10.0, Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
        assert_eq!(b.len(), 2);
        assert!(!b[0].fill && b[0].stroke);
        assert!(b[1].fill && !b[1].stroke, "darken paints like norm");
    }

    #[test]
    fn evaluation_failure_degrades_whole_geometry() {
        let g = geom(vec![path(
            Some(10),
            Some(10),
            vec![PathCmd::MoveTo(pt("0", "undefined_name"))],
        )]);
        assert!(build_paths(&g, 10.0, 10.0, Rect::new(0.0, 0.0, 10.0, 10.0)).is_none());
        let zero = CustGeom {
            gd_lst: vec![guide("a", "*/ w 1 0")],
            paths: vec![path(
                Some(10),
                Some(10),
                vec![PathCmd::MoveTo(pt("0", "0"))],
            )],
            ..CustGeom::default()
        };
        assert!(build_paths(&zero, 10.0, 10.0, Rect::new(0.0, 0.0, 10.0, 10.0)).is_none());
    }

    #[test]
    fn degenerate_inputs_never_panic() {
        // 路径 w / h 为 0 或负、lnTo 无当前点、arcTo 无当前点、半径 0
        let g = geom(vec![path(
            Some(0),
            Some(-5),
            vec![
                PathCmd::LnTo(pt("1", "1")),
                PathCmd::ArcTo {
                    w_r: "0".into(),
                    h_r: "0".into(),
                    st_ang: "0".into(),
                    sw_ang: "5400000".into(),
                },
                PathCmd::Close,
                PathCmd::Close,
            ],
        )]);
        let _ = build_paths(&g, 0.0, 0.0, Rect::new(0.0, 0.0, 0.0, 0.0));
        let _ = build_paths(&g, 10.0, 10.0, Rect::new(0.0, 0.0, 10.0, 10.0));
    }
}
