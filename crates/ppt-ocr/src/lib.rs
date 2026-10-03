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

use std::panic::{catch_unwind, AssertUnwindSafe};

use ocrspine::{OcrEngine, OcrError, OcrImage, OcrWord, PaddleOcr};
use ppt_core::{PptError, Result};

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

/// 在 panic 隔离下运行 `f`:任何 panic 都折成 [`PptError::Ocr`](带 payload 字符串),
/// 避免第三方引擎/解码器的 panic 穿过 FFI 变成 Python 的 `PanicException`(`except Exception`
/// 接不住)。做法同 pdfspine ADR 0006。
fn guard_panic<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|payload| {
        let msg = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic payload".to_string());
        Err(PptError::Ocr(format!("OCR engine panicked: {msg}")))
    })
}

/// 词的 bbox 与置信度是否全为有限数(下游几何与排序都假定有限数)。
fn is_finite_word(w: &OcrWord) -> bool {
    w.confidence.is_finite()
        && w.bbox.x0.is_finite()
        && w.bbox.y0.is_finite()
        && w.bbox.x1.is_finite()
        && w.bbox.y1.is_finite()
}

/// 调引擎识别并清洗结果:panic 隔离 + 丢弃含 NaN/Inf 的词。
fn recognize_guarded<E: OcrEngine>(engine: &E, image: &OcrImage) -> Result<Vec<OcrWord>> {
    let mut words = guard_panic(|| engine.recognize(image).map_err(map_ocr_err))?;
    words.retain(is_finite_word);
    Ok(words)
}

/// 解码图片字节(同样受 panic 隔离)。
fn decode_image(bytes: &[u8]) -> Result<OcrImage> {
    guard_panic(|| OcrImage::from_encoded(bytes).map_err(map_ocr_err))
}

/// 构造默认引擎(同样受 panic 隔离)。
fn new_engine() -> Result<PaddleOcr> {
    guard_panic(|| PaddleOcr::new().map_err(map_ocr_err))
}

/// 一次性 OCR:解码图片字节 -> 新建引擎 -> 识别 -> 映射。
///
/// 注意:每次调用都新建一个 [`PaddleOcr`]。这对**单张**图片是最简路径;批量图片请用
/// [`PptOcr`] 缓存引擎,避免重复构造。引擎 panic 折成 [`PptError::Ocr`];非有限数的词被丢弃。
pub fn ocr_image_bytes(bytes: &[u8]) -> Result<Vec<OcrItem>> {
    let image = decode_image(bytes)?;
    let engine = new_engine()?;
    let words = recognize_guarded(&engine, &image)?;
    Ok(words.into_iter().map(OcrItem::from).collect())
}

/// 跨多次调用缓存 [`PaddleOcr`] 引擎的 OCR 器(批量图片时复用)。
pub struct PptOcr {
    engine: PaddleOcr,
}

impl PptOcr {
    /// 新建一个缓存引擎的 OCR 器。
    pub fn new() -> Result<Self> {
        Ok(PptOcr {
            engine: new_engine()?,
        })
    }

    /// 对一张图片字节做 OCR,复用已缓存的引擎。
    pub fn ocr(&self, bytes: &[u8]) -> Result<Vec<OcrItem>> {
        let image = decode_image(bytes)?;
        let words = recognize_guarded(&self.engine, &image)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use ocrspine::BBox;

    fn word(x0: f64, conf: f32) -> OcrWord {
        OcrWord {
            text: "w".into(),
            bbox: BBox::new(x0, 0.0, x0 + 10.0, 10.0),
            confidence: conf,
            quad: [(0.0, 0.0); 4],
        }
    }

    fn image() -> OcrImage {
        OcrImage::from_rgb(1, 1, vec![0, 0, 0]).unwrap()
    }

    enum Mock {
        PanicStr,
        PanicString,
        Words(Vec<OcrWord>),
    }

    impl OcrEngine for Mock {
        fn recognize(&self, _: &OcrImage) -> ocrspine::Result<Vec<OcrWord>> {
            match self {
                Mock::PanicStr => panic!("boom"),
                Mock::PanicString => panic!("{}", String::from("boom-owned")),
                Mock::Words(w) => Ok(w.clone()),
            }
        }
    }

    #[test]
    fn engine_panic_becomes_ppt_error_ocr() {
        for (m, needle) in [(Mock::PanicStr, "boom"), (Mock::PanicString, "boom-owned")] {
            match recognize_guarded(&m, &image()) {
                Err(PptError::Ocr(msg)) => assert!(msg.contains(needle), "{msg}"),
                other => panic!("期望 Err(PptError::Ocr),得到 {other:?}"),
            }
        }
    }

    #[test]
    fn non_finite_words_are_dropped() {
        let m = Mock::Words(vec![
            word(0.0, 90.0),
            word(f64::NAN, 90.0),
            word(f64::INFINITY, 90.0),
            word(5.0, f32::NAN),
            word(6.0, f32::INFINITY),
            word(20.0, 80.0),
        ]);
        let got = recognize_guarded(&m, &image()).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].bbox.x0, 0.0);
        assert_eq!(got[1].bbox.x0, 20.0);
    }
}
