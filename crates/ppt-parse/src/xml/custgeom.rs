//! `a:custGeom` 解析:`avLst` / `gdLst` 参考线 + `pathLst > path` 路径命令 -> [`CustGeom`]。
//!
//! 只做结构抽取(坐标 / 公式参数保持原始字符串),求值在渲染侧。带资源预算
//! ([`MAX_GUIDES`] / [`MAX_PATHS`] / [`MAX_PATH_COMMANDS`]):超限整个几何丢弃
//! (调用方记诊断、渲染退回包围盒),但仍读到 `a:custGeom` 结束标签,不让父级解析失步。

use ppt_core::custgeom::{
    CustGeom, CustPath, CustPt, Guide, PathCmd, PathFill, MAX_GUIDES, MAX_PATHS, MAX_PATH_COMMANDS,
};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::{attr_of, bool_attr, local_name, ooxml_bool, skip_element};

/// 解析结果。
pub(super) enum Parsed {
    /// 结构完整、在预算内且至少有一条路径。
    Geom(CustGeom),
    /// 超预算(已读到结束标签)。
    OverBudget,
    /// 没有可用路径(空元素 / 无 `pathLst`):退回旧的包围盒近似。
    Empty,
}

/// 解析 `a:custGeom`。已消费其起始标签,消费到其结束标签。
pub(super) fn parse_cust_geom<R: std::io::BufRead>(reader: &mut Reader<R>) -> Parsed {
    let mut geom = CustGeom::default();
    let mut guides = 0usize;
    let mut cmds = 0usize;
    let mut over = false;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"avLst" | b"gdLst" => {
                        let list = parse_guides(reader, &mut guides, &mut over);
                        if name.as_slice() == b"avLst" {
                            geom.av_lst.extend(list);
                        } else {
                            geom.gd_lst.extend(list);
                        }
                    }
                    b"pathLst" => parse_path_lst(reader, &mut geom, &mut cmds, &mut over),
                    _ => skip_element(reader, &name),
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    if over {
        Parsed::OverBudget
    } else if geom.paths.is_empty() {
        Parsed::Empty
    } else {
        Parsed::Geom(geom)
    }
}

/// `a:avLst` / `a:gdLst` 内的 `a:gd`。`guides` 是跨两个列表的累计数;超 [`MAX_GUIDES`] 置 `over`
/// 并停止收集(仍读完该列表)。
fn parse_guides<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    guides: &mut usize,
    over: &mut bool,
) -> Vec<Guide> {
    let mut out = Vec::new();
    let mut depth = 1usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                take_guide(&e, &mut out, guides, over);
            }
            Ok(Event::Empty(e)) => take_guide(&e, &mut out, guides, over),
            Ok(Event::End(_)) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

fn take_guide(e: &BytesStart, out: &mut Vec<Guide>, guides: &mut usize, over: &mut bool) {
    if local_name(e.name().as_ref()) != b"gd" || *over {
        return;
    }
    let (Some(name), Some(fmla)) = (attr_of(e, b"name"), attr_of(e, b"fmla")) else {
        return;
    };
    if *guides >= MAX_GUIDES {
        *over = true;
        out.clear();
        return;
    }
    *guides += 1;
    out.push(Guide { name, fmla });
}

fn parse_path_lst<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    geom: &mut CustGeom,
    cmds: &mut usize,
    over: &mut bool,
) {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref()).to_vec();
                if name.as_slice() == b"path" {
                    let mut path = path_of(&e);
                    parse_path_body(reader, &mut path, cmds, over);
                    if geom.paths.len() >= MAX_PATHS {
                        *over = true;
                    } else if !*over {
                        geom.paths.push(path);
                    }
                } else {
                    skip_element(reader, &name);
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
}

fn path_of(e: &BytesStart) -> CustPath {
    let dim = |key: &[u8]| attr_of(e, key).and_then(|v| v.trim().parse::<i64>().ok());
    let fill = match attr_of(e, b"fill").as_deref() {
        Some("none") => PathFill::None,
        Some("lighten") => PathFill::Lighten,
        Some("lightenLess") => PathFill::LightenLess,
        Some("darken") => PathFill::Darken,
        Some("darkenLess") => PathFill::DarkenLess,
        _ => PathFill::Norm,
    };
    CustPath {
        w: dim(b"w"),
        h: dim(b"h"),
        fill,
        stroke: attr_of(e, b"stroke").is_none_or(ooxml_bool),
        extrusion_ok: bool_attr(e, b"extrusionOk"),
        cmds: Vec::new(),
    }
}

/// `a:path` 内的命令,读到 `</a:path>`。命令总数超 [`MAX_PATH_COMMANDS`] 置 `over`。
fn parse_path_body<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    path: &mut CustPath,
    cmds: &mut usize,
    over: &mut bool,
) {
    let mut buf = Vec::new();
    loop {
        let ev = reader.read_event_into(&mut buf);
        let (e, has_children) = match &ev {
            Ok(Event::Start(e)) => (e.clone(), true),
            Ok(Event::Empty(e)) => (e.clone(), false),
            Ok(Event::End(_)) | Ok(Event::Eof) | Err(_) => break,
            _ => {
                buf.clear();
                continue;
            }
        };
        let name = local_name(e.name().as_ref()).to_vec();
        let pts = if has_children {
            read_pts(reader)
        } else {
            Vec::new()
        };
        let cmd = match (name.as_slice(), pts.as_slice()) {
            (b"moveTo", [p]) => Some(PathCmd::MoveTo(p.clone())),
            (b"lnTo", [p]) => Some(PathCmd::LnTo(p.clone())),
            (b"cubicBezTo", [a, b, c]) => {
                Some(PathCmd::CubicBezTo(a.clone(), b.clone(), c.clone()))
            }
            (b"quadBezTo", [a, b]) => Some(PathCmd::QuadBezTo(a.clone(), b.clone())),
            (b"arcTo", _) => Some(PathCmd::ArcTo {
                w_r: attr_of(&e, b"wR").unwrap_or_default(),
                h_r: attr_of(&e, b"hR").unwrap_or_default(),
                st_ang: attr_of(&e, b"stAng").unwrap_or_default(),
                sw_ang: attr_of(&e, b"swAng").unwrap_or_default(),
            }),
            (b"close", _) => Some(PathCmd::Close),
            _ => None,
        };
        if let Some(c) = cmd {
            if *cmds >= MAX_PATH_COMMANDS {
                *over = true;
            } else if !*over {
                *cmds += 1;
                path.cmds.push(c);
            }
        }
        buf.clear();
    }
}

/// 读一个命令元素内的 `a:pt`(已消费命令起始标签;读到其结束标签)。
fn read_pts<R: std::io::BufRead>(reader: &mut Reader<R>) -> Vec<CustPt> {
    let mut pts = Vec::new();
    let mut depth = 1usize;
    let mut buf = Vec::new();
    let take = |e: &BytesStart, pts: &mut Vec<CustPt>| {
        if local_name(e.name().as_ref()) == b"pt" {
            pts.push(CustPt {
                x: attr_of(e, b"x").unwrap_or_default(),
                y: attr_of(e, b"y").unwrap_or_default(),
            });
        }
    };
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                take(&e, &mut pts);
            }
            Ok(Event::Empty(e)) => take(&e, &mut pts),
            Ok(Event::End(_)) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    pts
}
