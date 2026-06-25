#![forbid(unsafe_code)]
//! `ppt-ocr` —— 图片 OCR 桥。
//!
//! 把姊妹 crate [`ocrspine`] 的 PP-OCRv5(`tract-onnx`,本地、离线、确定性)套到 pptx
//! 嵌入图片上。这是缝的元模式:`OcrEngine` 协议来自 ocrspine,`PaddleOcr` 是确定性默认实现;
//! 本 crate 只把字节喂进去、把结果 [`OcrWord`] 映射成本地的 [`OcrItem`],并把
//! [`ocrspine::OcrError`] 折成 [`PptError::Ocr`]。
//!
//! 本轮**逐图 OCR 真正可用**;基于 OCR 框做表格行列几何重建是后续工作,见
//! [`reconstruct_table_from_image`](fn@reconstruct_table_from_image) 的 stub。

use ppt_core::{PptError, Result};
use ocrspine::{OcrEngine, OcrError, OcrImage, OcrWord, PaddleOcr};

/// 一条 OCR 结果:文字 + 轴对齐外框 + 置信度。坐标原点在图片左上角,y 向下。
#[derive(Debug, Clone, PartialEq)]
pub struct OcrItem {
    pub text: String,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    /// 置信度(`[0.0, 100.0]` 标度,沿用 ocrspine)。
    pub confidence: f32,
}

impl From<OcrWord> for OcrItem {
    fn from(w: OcrWord) -> Self {
        OcrItem {
            text: w.text,
            x0: w.bbox.x0,
            y0: w.bbox.y0,
            x1: w.bbox.x1,
            y1: w.bbox.y1,
            confidence: w.confidence,
        }
    }
}

/// 把 ocrspine 的错误折成本地 [`PptError::Ocr`]。
fn map_ocr_err(e: OcrError) -> PptError {
    PptError::Ocr(e.to_string())
}

/// 一次性 OCR:解码图片字节 -> 新建引擎 -> 识别 -> 映射。
///
/// 注意:每次调用都新建一个 [`PaddleOcr`]。这对**单张**图片是最简路径;批量图片请用
/// [`PptOcr`] 缓存引擎,避免重复构造。
pub fn ocr_image_bytes(bytes: &[u8]) -> Result<Vec<OcrItem>> {
    let image = OcrImage::from_encoded(bytes).map_err(map_ocr_err)?;
    let engine = PaddleOcr::new().map_err(map_ocr_err)?;
    let words = engine.recognize(&image).map_err(map_ocr_err)?;
    Ok(words.into_iter().map(OcrItem::from).collect())
}

/// 跨多次调用缓存 [`PaddleOcr`] 引擎的 OCR 器(批量图片时复用)。
pub struct PptOcr {
    engine: PaddleOcr,
}

impl PptOcr {
    /// 新建一个缓存引擎的 OCR 器。
    pub fn new() -> Result<Self> {
        let engine = PaddleOcr::new().map_err(map_ocr_err)?;
        Ok(PptOcr { engine })
    }

    /// 对一张图片字节做 OCR,复用已缓存的引擎。
    pub fn ocr(&self, bytes: &[u8]) -> Result<Vec<OcrItem>> {
        let image = OcrImage::from_encoded(bytes).map_err(map_ocr_err)?;
        let words = self.engine.recognize(&image).map_err(map_ocr_err)?;
        Ok(words.into_iter().map(OcrItem::from).collect())
    }
}

/// **[STUB / 延后]** 从一张图片重建表格(行列几何 + 单元格文字)。
///
/// 把 OCR 出来的文字框聚类成行/列、推断网格、回填单元格,是一块独立的几何重建工作,
/// 留作后续。当前一律返回 [`PptError::Unsupported`]。逐图 OCR([`ocr_image_bytes`] /
/// [`PptOcr::ocr`])已端到端可用,本函数不影响它。
pub fn reconstruct_table_from_image(_bytes: &[u8]) -> Result<()> {
    Err(PptError::Unsupported(
        "reconstruct_table_from_image: image-table geometry reconstruction is deferred; \
         per-image OCR via ocr_image_bytes / PptOcr::ocr works today"
            .into(),
    ))
}
