// `py-bindings` 是唯一的 FFI 关隘,也是唯一允许使用 `unsafe` 的一方(PyO3 会生成 FFI
// glue)。因此它不 `forbid(unsafe_code)`,而是要求 `unsafe` 必须被显式限定作用域。
#![deny(unsafe_op_in_unsafe_fn)]
//! 把 pptspine 的 Rust 核暴露给 Python 的 `_core` 扩展模块(PyO3 / maturin,abi3-py311)。
//!
//! 暴露**只读**解析面:`open` / `open_bytes` 返回一个 [`Presentation`] 句柄,其上可取
//! `slide_count` / `slide_size` 与 `slides()`;每张 [`Slide`] 的 `shapes()` 返回
//! `list[dict]`(可自省、稳定的形状)。外加 `ocr_image` 把图片字节交给姊妹 crate
//! `ocrspine`(PP-OCRv5,本地、离线、确定性)做 OCR。
//!
//! **句柄/索引模式**:每个 `#[pyclass]` 都是 `'static` 且各自持有 `Arc` 共享的已解析数据,
//! 绝不持有 Rust 借用。重活(解析 / OCR)在 [`Python::detach`] 下释放 GIL 运行。错误折成
//! 以 `_core.PptError` 为根的类型化异常层级。

use std::path::PathBuf;
use std::sync::Arc;

use ppt_core::geom::emu_to_points;
use ppt_core::model::{
    AutoShape, Cell, Color, Paragraph, Picture, Presentation as CorePresentation, Row, Shape,
    Slide as CoreSlide, Table, TextFrame, TextRun,
};
use ppt_core::PptError;
use ppt_ocr::{ocr_image_bytes, OcrItem};
use ppt_parse::{parse_bytes, parse_path};
use pyo3::create_exception;
use pyo3::exceptions::{PyFileNotFoundError, PyIndexError, PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

/// 包版本(镜像 Rust workspace 版本)。
const VERSION: &str = env!("CARGO_PKG_VERSION");

// --- 异常层级 -------------------------------------------------------------

create_exception!(_core, PptError_, pyo3::exceptions::PyException);
create_exception!(_core, PptZipError, PptError_);
create_exception!(_core, PptXmlError, PptError_);
create_exception!(_core, PptUnsupportedError, PptError_);
create_exception!(_core, PptOcrError, PptError_);

/// 把 [`PptError`] 折成对应的 Python 异常(按 `kind()` 稳定标签分派)。
fn map_err(e: PptError) -> PyErr {
    let msg = e.to_string();
    match e.kind() {
        "zip" => PptZipError::new_err(msg),
        "xml" => PptXmlError::new_err(msg),
        "unsupported" => PptUnsupportedError::new_err(msg),
        "ocr" => PptOcrError::new_err(msg),
        "invalid-argument" => PyValueError::new_err(msg),
        "io" => {
            if let PptError::Io(io) = &e {
                if io.kind() == std::io::ErrorKind::NotFound {
                    return PyFileNotFoundError::new_err(msg);
                }
            }
            PyOSError::new_err(msg)
        }
        _ => PptError_::new_err(msg),
    }
}

// --- 颜色 / 几何小工具 ----------------------------------------------------

/// 把一个 [`Color`] 转成 `"RRGGBB"` 十六进制串。
fn color_hex(c: &Color) -> String {
    format!("{:02X}{:02X}{:02X}", c.rgb[0], c.rgb[1], c.rgb[2])
}

/// 把一个可选 [`ppt_core::geom::Rect`] 转成 `(x, y, w, h)` 磅(point)四元组的 dict
/// 字段,缺失时为 `None`。返回 `(emu_tuple, points_tuple)`。
fn rect_to_py(
    py: Python<'_>,
    rect: Option<ppt_core::geom::Rect>,
) -> (Option<Py<PyAny>>, Option<Py<PyAny>>) {
    match rect {
        Some(r) => {
            let emu = (r.x, r.y, r.w, r.h).into_pyobject(py).map(|b| b.into());
            let pts = (
                emu_to_points(r.x),
                emu_to_points(r.y),
                emu_to_points(r.w),
                emu_to_points(r.h),
            )
                .into_pyobject(py)
                .map(|b| b.into());
            (emu.ok(), pts.ok())
        }
        None => (None, None),
    }
}

// --- dict 构造:把领域模型映射成可自省的 list[dict] ----------------------

/// 一个 [`TextRun`] -> dict。
fn run_dict<'py>(py: Python<'py>, run: &TextRun) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("text", &run.text)?;
    d.set_item("font", run.font.as_deref())?;
    d.set_item("size_pt", run.size_pt)?;
    d.set_item("bold", run.bold)?;
    d.set_item("italic", run.italic)?;
    d.set_item("color", run.color.as_ref().map(color_hex))?;
    Ok(d)
}

/// 一个 [`Paragraph`] -> dict。
fn paragraph_dict<'py>(py: Python<'py>, para: &Paragraph) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    let runs = PyList::empty(py);
    for r in &para.runs {
        runs.append(run_dict(py, r)?)?;
    }
    // 便利字段:整段拼接文字。
    let text: String = para.runs.iter().map(|r| r.text.as_str()).collect();
    d.set_item("runs", runs)?;
    d.set_item("text", text)?;
    d.set_item("level", para.level)?;
    d.set_item("align", para.align.as_deref())?;
    Ok(d)
}

/// 把一串段落映射成 `list[dict]`,并拼出整体文字便利串。
fn paragraphs_py<'py>(
    py: Python<'py>,
    paragraphs: &[Paragraph],
) -> PyResult<(Bound<'py, PyList>, String)> {
    let list = PyList::empty(py);
    let mut texts: Vec<String> = Vec::new();
    for p in paragraphs {
        list.append(paragraph_dict(py, p)?)?;
        texts.push(p.runs.iter().map(|r| r.text.as_str()).collect());
    }
    Ok((list, texts.join("\n")))
}

/// 一个 [`TextFrame`] -> dict(供文本框 / autoshape 内嵌文字复用)。
fn text_frame_dict<'py>(py: Python<'py>, tf: &TextFrame) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    let (paras, text) = paragraphs_py(py, &tf.paragraphs)?;
    let (rect_emu, rect_pts) = rect_to_py(py, tf.rect);
    d.set_item("kind", "text")?;
    d.set_item("rect", rect_emu)?;
    d.set_item("rect_points", rect_pts)?;
    d.set_item("paragraphs", paras)?;
    d.set_item("text", text)?;
    Ok(d)
}

/// 一个 [`Cell`] -> dict。
fn cell_dict<'py>(py: Python<'py>, cell: &Cell) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    let (paras, text) = paragraphs_py(py, &cell.paragraphs)?;
    d.set_item("paragraphs", paras)?;
    d.set_item("text", text)?;
    d.set_item("col_span", cell.col_span)?;
    d.set_item("row_span", cell.row_span)?;
    d.set_item("fill", cell.fill.as_ref().map(color_hex))?;
    d.set_item("merged", cell.merged)?;
    Ok(d)
}

/// 一个 [`Row`] -> dict(`cells` + 便利的 `text` 列表)。
fn row_dict<'py>(py: Python<'py>, row: &Row) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    let cells = PyList::empty(py);
    let texts = PyList::empty(py);
    for c in &row.cells {
        let cd = cell_dict(py, c)?;
        texts.append(cd.get_item("text")?)?;
        cells.append(cd)?;
    }
    d.set_item("cells", cells)?;
    d.set_item("text", texts)?;
    d.set_item("height", row.height)?;
    Ok(d)
}

/// 一张 [`Table`] -> dict。
fn table_dict<'py>(py: Python<'py>, table: &Table) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    let rows = PyList::empty(py);
    for r in &table.rows {
        rows.append(row_dict(py, r)?)?;
    }
    let (rect_emu, rect_pts) = rect_to_py(py, table.rect);
    d.set_item("kind", "table")?;
    d.set_item("rect", rect_emu)?;
    d.set_item("rect_points", rect_pts)?;
    d.set_item("rows", rows)?;
    Ok(d)
}

/// 一张 [`Picture`] -> dict。
fn picture_dict<'py>(py: Python<'py>, pic: &Picture) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    let (rect_emu, rect_pts) = rect_to_py(py, pic.rect);
    d.set_item("kind", "picture")?;
    d.set_item("rect", rect_emu)?;
    d.set_item("rect_points", rect_pts)?;
    d.set_item("rel_id", &pic.rel_id)?;
    d.set_item("media", pic.media_name.as_deref())?;
    d.set_item("image_bytes_len", pic.image_bytes_len)?;
    Ok(d)
}

/// 一个 [`AutoShape`] -> dict。
fn autoshape_dict<'py>(py: Python<'py>, sh: &AutoShape) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    let (rect_emu, rect_pts) = rect_to_py(py, sh.rect);
    d.set_item("kind", "auto")?;
    d.set_item("rect", rect_emu)?;
    d.set_item("rect_points", rect_pts)?;
    d.set_item("geometry", sh.geometry.as_deref())?;
    d.set_item("fill", sh.fill.as_ref().map(color_hex))?;
    d.set_item("stroke", sh.stroke.as_ref().map(color_hex))?;
    match &sh.text {
        Some(tf) => {
            let (paras, text) = paragraphs_py(py, &tf.paragraphs)?;
            d.set_item("paragraphs", paras)?;
            d.set_item("text", text)?;
        }
        None => {
            d.set_item("paragraphs", PyList::empty(py))?;
            d.set_item("text", py.None())?;
        }
    }
    Ok(d)
}

/// 一个 [`Shape`] -> dict(组合递归到 `children`)。
fn shape_dict<'py>(py: Python<'py>, shape: &Shape) -> PyResult<Bound<'py, PyDict>> {
    match shape {
        Shape::TextBox(tf) => text_frame_dict(py, tf),
        Shape::Table(t) => table_dict(py, t),
        Shape::Picture(p) => picture_dict(py, p),
        Shape::Auto(a) => autoshape_dict(py, a),
        Shape::Group(children) => {
            let d = PyDict::new(py);
            let kids = PyList::empty(py);
            for c in children {
                kids.append(shape_dict(py, c)?)?;
            }
            d.set_item("kind", "group")?;
            d.set_item("children", kids)?;
            Ok(d)
        }
    }
}

// --- pyclass 句柄 ---------------------------------------------------------

/// 一份已解析的演示文稿句柄(`Arc` 共享底层数据)。
#[pyclass(name = "Presentation", module = "pptspine._core", frozen)]
struct PyPresentation {
    inner: Arc<CorePresentation>,
}

#[pymethods]
impl PyPresentation {
    /// 幻灯片数量。
    #[getter]
    fn slide_count(&self) -> usize {
        self.inner.slides.len()
    }

    /// 画布尺寸 `(cx, cy)`(EMU)。
    #[getter]
    fn slide_size(&self) -> (i64, i64) {
        self.inner.slide_size
    }

    /// 画布尺寸 `(w, h)`(磅)。
    #[getter]
    fn slide_size_points(&self) -> (f64, f64) {
        let (cx, cy) = self.inner.slide_size;
        (emu_to_points(cx), emu_to_points(cy))
    }

    /// 所有幻灯片句柄。
    fn slides(&self) -> Vec<PySlide> {
        (0..self.inner.slides.len())
            .map(|i| PySlide {
                pres: Arc::clone(&self.inner),
                index: i,
            })
            .collect()
    }

    /// 按序号取一张幻灯片(越界抛 `IndexError`)。
    fn slide(&self, index: usize) -> PyResult<PySlide> {
        if index >= self.inner.slides.len() {
            return Err(PyIndexError::new_err(format!(
                "slide index {index} out of range (slide_count = {})",
                self.inner.slides.len()
            )));
        }
        Ok(PySlide {
            pres: Arc::clone(&self.inner),
            index,
        })
    }

    fn __len__(&self) -> usize {
        self.inner.slides.len()
    }

    fn __repr__(&self) -> String {
        let (cx, cy) = self.inner.slide_size;
        format!(
            "<pptspine.Presentation slide_count={} slide_size=({cx}, {cy})>",
            self.inner.slides.len()
        )
    }
}

/// 一张幻灯片句柄(共享演示文稿数据 + 自身序号)。
#[pyclass(name = "Slide", module = "pptspine._core", frozen)]
struct PySlide {
    pres: Arc<CorePresentation>,
    index: usize,
}

impl PySlide {
    /// 取底层 core slide(序号在构造期已保证有效)。
    fn core(&self) -> &CoreSlide {
        &self.pres.slides[self.index]
    }
}

#[pymethods]
impl PySlide {
    /// 零基序号。
    #[getter]
    fn index(&self) -> usize {
        self.index
    }

    /// 关联版式名(best-effort)。
    #[getter]
    fn layout_name(&self) -> Option<String> {
        self.core().layout_name.clone()
    }

    /// 关联母版名(best-effort)。
    #[getter]
    fn master_name(&self) -> Option<String> {
        self.core().master_name.clone()
    }

    /// 顶层形状,作为 `list[dict]`。
    fn shapes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let list = PyList::empty(py);
        for sh in &self.core().shapes {
            list.append(shape_dict(py, sh)?)?;
        }
        Ok(list)
    }

    fn __repr__(&self) -> String {
        format!(
            "<pptspine.Slide index={} shapes={}>",
            self.index,
            self.core().shapes.len()
        )
    }
}

// --- 模块级函数 -----------------------------------------------------------

/// 从磁盘路径解析一个 `.pptx`。解析在释放 GIL 下进行。
#[pyfunction]
fn open(py: Python<'_>, path: PathBuf) -> PyResult<PyPresentation> {
    let parsed = py.detach(|| parse_path(&path)).map_err(map_err)?;
    Ok(PyPresentation {
        inner: Arc::new(parsed.presentation),
    })
}

/// 从内存字节解析一个 `.pptx`。解析在释放 GIL 下进行。
#[pyfunction]
fn open_bytes(py: Python<'_>, data: &[u8]) -> PyResult<PyPresentation> {
    let owned = data.to_vec();
    let parsed = py.detach(|| parse_bytes(&owned)).map_err(map_err)?;
    Ok(PyPresentation {
        inner: Arc::new(parsed.presentation),
    })
}

/// 把 [`OcrItem`] 折成一个 dict。
fn ocr_item_dict<'py>(py: Python<'py>, it: &OcrItem) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("text", &it.text)?;
    d.set_item("bbox", (it.x0, it.y0, it.x1, it.y1))?;
    d.set_item("confidence", it.confidence)?;
    Ok(d)
}

/// 对一张图片的编码字节(PNG / JPEG / TIFF / BMP)做本地 OCR,返回 `list[dict]`,每项含
/// `text` / `bbox` / `confidence`。推理在释放 GIL 下进行(本地、离线、确定性)。
#[pyfunction]
fn ocr_image<'py>(py: Python<'py>, data: &[u8]) -> PyResult<Bound<'py, PyList>> {
    let owned = data.to_vec();
    let items = py.detach(|| ocr_image_bytes(&owned)).map_err(map_err)?;
    let list = PyList::empty(py);
    for it in &items {
        list.append(ocr_item_dict(py, it)?)?;
    }
    Ok(list)
}

/// 包版本。
#[pyfunction]
fn version() -> &'static str {
    VERSION
}

// --- 模块注册 -------------------------------------------------------------

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("__version__", VERSION)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(open, m)?)?;
    m.add_function(wrap_pyfunction!(open_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(ocr_image, m)?)?;

    m.add_class::<PyPresentation>()?;
    m.add_class::<PySlide>()?;

    // 异常层级(根 `PptError`)。`PptError_` 的 Rust 标识符带下划线避免与
    // `ppt_core::PptError` 撞名,但暴露给 Python 的名字是 `PptError`。
    m.add("PptError", py.get_type::<PptError_>())?;
    m.add("PptZipError", py.get_type::<PptZipError>())?;
    m.add("PptXmlError", py.get_type::<PptXmlError>())?;
    m.add("PptUnsupportedError", py.get_type::<PptUnsupportedError>())?;
    m.add("PptOcrError", py.get_type::<PptOcrError>())?;

    Ok(())
}
