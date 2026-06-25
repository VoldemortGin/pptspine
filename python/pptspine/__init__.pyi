"""``pptspine`` 顶层 API 的类型存根(PEP 561)。"""

from __future__ import annotations

from ._core import (
    Presentation as Presentation,
    PptError as PptError,
    PptOcrError as PptOcrError,
    PptUnsupportedError as PptUnsupportedError,
    PptXmlError as PptXmlError,
    PptZipError as PptZipError,
    Slide as Slide,
    ocr_image as ocr_image,
    open as open,
    open_bytes as open_bytes,
)

__version__: str

__all__ = [
    "Presentation",
    "Slide",
    "open",
    "open_bytes",
    "ocr_image",
    "PptError",
    "PptZipError",
    "PptXmlError",
    "PptUnsupportedError",
    "PptOcrError",
    "__version__",
]
