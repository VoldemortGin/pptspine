"""pptspine —— 纯 Rust 的 PowerPoint(.pptx / OOXML)结构化解析器 + 本地图片 OCR。

这是由 Rust ``_core`` 扩展模块(PyO3 / maturin,abi3-py311)支撑的 Python 包。
解析面只读::func:`open` / :func:`open_bytes` 返回一个 :class:`Presentation` 句柄,
其上可取 ``slide_count`` / ``slide_size`` 与 ``slides()``;每张 :class:`Slide` 的
``shapes()`` 返回 ``list[dict]``(可自省、稳定的形状)。

:func:`ocr_image` 把图片字节交给姊妹 crate ``ocrspine``(PP-OCRv5,本地、离线、确定性)
做 OCR —— 无云端、无网络。
"""

from __future__ import annotations

from importlib.metadata import PackageNotFoundError, version as _pkg_version

from . import _core
from ._core import (
    Presentation,
    PptError,
    PptOcrError,
    PptUnsupportedError,
    PptXmlError,
    PptZipError,
    Slide,
    ocr_image,
    open,
    open_bytes,
)

try:
    __version__ = _pkg_version("pptspine")
except PackageNotFoundError:  # 源码树里未安装时回退到扩展自带版本。
    __version__ = _core.__version__

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
