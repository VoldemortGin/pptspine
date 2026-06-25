"""图片 OCR 桥的验收测试 —— 把已知图片喂给 ``pptspine.ocr_image`` 断言识别结果。

OCR 走姊妹 crate ``ocrspine``(PP-OCRv5 / tract-onnx,本地、离线、确定性)。``ocr_image``
是顶层 Python 包装,委托 Rust ``_core`` 前先把引擎指向 wheel 内自带的 ``pptspine/_models``
权重(或由 ``PPTSPINE_OCR_MODELS`` / ``OCRSPINE_MODELS`` 环境变量覆盖,或源码 checkout 里
回退到 ocrspine 编译期烘进的 ``CARGO_MANIFEST_DIR/models``),所以默认离线即可跑。复用
本仓自带、已验证的 ``ocr_sample.png`` fixture(含 "pdfspine OCR test 2026" 等参考行)。
"""

from __future__ import annotations

from pathlib import Path

import pytest

import pptspine

# OCR 样张随仓 vendored 在 python/tests/fixtures/(与 ocrspine 的字节一致),使测试不依赖
# 姊妹 ocrspine clone 即可跑。
_OCR_SAMPLE = Path(__file__).resolve().parent / "fixtures" / "ocr_sample.png"


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
