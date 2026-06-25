//! `ppt-ocr` 桥的轻量测试:断言坏字节走错误映射(`PptError::Ocr`),不依赖模型、绝不 panic。
//!
//! 真正的端到端识别(喂真实图片、加载 ONNX 模型)由 Python 测试 `test_ocr.py` 覆盖;
//! 这里只钉住「桥不 panic、错误被折成类型化 Ocr 变体」这条不变量。

use ppt_core::PptError;
use ppt_ocr::ocr_image_bytes;

#[test]
fn bad_image_bytes_map_to_ocr_error() {
    let err = ocr_image_bytes(b"definitely not an image").unwrap_err();
    assert_eq!(err.kind(), "ocr");
    assert!(matches!(err, PptError::Ocr(_)));
}
