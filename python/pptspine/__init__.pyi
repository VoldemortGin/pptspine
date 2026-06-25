"""``pptspine`` 顶层 API 的类型存根(PEP 561)。"""

from __future__ import annotations

from typing import Any

from ._core import (
    Presentation as Presentation,
    PptError as PptError,
    PptOcrError as PptOcrError,
    PptUnsupportedError as PptUnsupportedError,
    PptXmlError as PptXmlError,
    PptZipError as PptZipError,
    Slide as Slide,
    open as open,
    open_bytes as open_bytes,
)

# ``ocr_image`` 是顶层 Python 包装:委托给 ``_core.ocr_image`` 前先把引擎指向
# wheel 内自带的 PP-OCRv5 权重(见 ``__init__.py`` 的 ``_ensure_ocr_models_env``)。
def ocr_image(data: bytes) -> list[dict[str, Any]]: ...

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
