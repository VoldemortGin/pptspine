"""图片 OCR 桥的验收测试 —— 把已知图片喂给 ``pptspine.ocr_image`` 断言识别结果。

OCR 走姊妹 crate ``ocrspine``(PP-OCRv5 / tract-onnx,本地、离线、确定性)。模型由扩展
在编译期烘进的 ``CARGO_MANIFEST_DIR/models``(即 ``../ocrspine/models``)解析,或由
``OCRSPINE_MODELS`` 环境变量覆盖,所以默认离线即可跑。复用 ocrspine 自带、已验证的
``ocr_sample.png`` fixture(含 "pdfspine OCR test 2026" 等参考行),不另落二进制。
"""

from __future__ import annotations

from pathlib import Path

import pytest

import pptspine

# ocrspine 是 pptspine 的姊妹包,布局为 spine/ocrspine 与 spine/pptspine 平级。
# 从本测试文件往上回到 spine/,再进 ocrspine 的 fixture。
_OCR_SAMPLE = (
    Path(__file__).resolve().parents[3] / "ocrspine" / "tests" / "fixtures" / "ocr_sample.png"
)


@pytest.fixture(scope="session")
def ocr_sample_bytes() -> bytes:
    if not _OCR_SAMPLE.is_file():
        pytest.skip(f"OCR sample fixture not found at {_OCR_SAMPLE}")
    return _OCR_SAMPLE.read_bytes()


def test_ocr_image_recognizes_reference_lines(ocr_sample_bytes):
    items = pptspine.ocr_image(ocr_sample_bytes)
    assert isinstance(items, list)
    assert items, "OCR returned no items at all"

    # 每项是 {text, bbox, confidence}。
    first = items[0]
    assert set(first) == {"text", "bbox", "confidence"}
    assert isinstance(first["text"], str)
    assert len(first["bbox"]) == 4
    assert 0.0 <= first["confidence"] <= 100.0

    # 把所有识别文字去空白拼接,断言三条参考行各自出现。
    joined = "".join(ch for it in items for ch in it["text"] if not ch.isspace())
    for ref in ("pdfspineOCRtest2026", "纯Rust实现的PDF文字识别", "PaddleOCRviatract"):
        assert ref in joined, f"reference line {ref!r} not found in {joined!r}"


def test_ocr_image_bad_bytes_raises():
    # 非图片字节 -> 类型化 PptOcrError(PptError 子类),绝不 panic。
    with pytest.raises(pptspine.PptError):
        pptspine.ocr_image(b"not an image at all")
