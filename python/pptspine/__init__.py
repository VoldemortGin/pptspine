"""pptspine —— 纯 Rust 的 PowerPoint(.pptx / OOXML)结构化解析器 + 本地图片 OCR。

这是由 Rust ``_core`` 扩展模块(PyO3 / maturin,abi3-py311)支撑的 Python 包。
解析面只读::func:`open` / :func:`open_bytes` 返回一个 :class:`Presentation` 句柄,
其上可取 ``slide_count`` / ``slide_size`` 与 ``slides()``;每张 :class:`Slide` 的
``shapes()`` 返回 ``list[dict]``(可自省、稳定的形状)。

:func:`ocr_image` 把图片字节交给姊妹 crate ``ocrspine``(PP-OCRv5,本地、离线、确定性)
做 OCR —— 无云端、无网络。已发布的 ``pptspine`` wheel **编进**了 OCR 代码;~28 MB 的
PP-OCRv5 ONNX 权重来自共享数据包 ``ocrspine-models``(硬依赖),所以裸
``pip install pptspine`` 即全功能 OCR、离线可跑。
"""

from __future__ import annotations

import os
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
    open,
    open_bytes,
)
from ._core import ocr_image as _core_ocr_image

# --- OCR 模型解析:把 Rust PaddleOCR 引擎指向模型权重 ---------------------------

# pptspine-side 的文档化覆盖变量(用户显式指定模型目录时优先)。
_OCR_MODELS_ENV = "PPTSPINE_OCR_MODELS"
# OCR 推理在姊妹 crate ``ocrspine`` 里,其 ``models_dir()`` 读这个变量。我们把解析出
# 的目录镜像进它,使权重无需任何额外设置即可被找到。
_OCRSPINE_MODELS_ENV = "OCRSPINE_MODELS"


def _ensure_ocr_models_env() -> None:
    """把 Rust PaddleOCR 引擎指向模型权重(惰性、廉价、幂等、跨平台)。

    已发布的 ``pptspine`` wheel 编进了 OCR 代码但不带模型;~28 MB 的 PP-OCRv5 ONNX
    权重来自共享数据包 ``ocrspine-models``(硬依赖),所以裸 ``pip install pptspine``
    即全功能 OCR、离线可跑。OCR 推理在姊妹 crate ``ocrspine`` 里,其 ``models_dir()``
    读 ``OCRSPINE_MODELS``。调用 ``ocr_image`` 前先解析出一个模型目录,并同时导出为
    ``OCRSPINE_MODELS``(引擎实际读取的)与 ``PPTSPINE_OCR_MODELS``(pptspine 侧的文档
    化覆盖名)。

    解析顺序:

    1. ``PPTSPINE_OCR_MODELS`` 已在环境里 → 镜像进 ``OCRSPINE_MODELS``(用户覆盖);
    2. 否则已安装的共享数据包 ``ocrspine_models`` → 从其 ``models_dir()`` 设置两者
       (已装 wheel 的默认路径);
    3. 否则什么都不做 —— 引擎回退到编译期烘进的 ``ocrspine/models`` 开发目录(源码
       checkout),或抛出清晰的 ``PptOcrError`` / ``PptUnsupportedError``。
    """

    def _export(directory: str) -> None:
        os.environ[_OCR_MODELS_ENV] = directory
        os.environ[_OCRSPINE_MODELS_ENV] = directory

    override = os.environ.get(_OCR_MODELS_ENV)
    if override:
        # 尊重 pptspine 侧的显式覆盖:无条件镜像进引擎变量,使其确定性生效(盖掉上一次
        # 调用可能残留的 ``OCRSPINE_MODELS``)。
        os.environ[_OCRSPINE_MODELS_ENV] = override
        return
    try:
        import ocrspine_models
    except ImportError:
        return
    try:
        _export(os.fspath(ocrspine_models.models_dir()))
    except Exception:
        # 数据包损坏/不全时不要掩盖引擎自身的清晰报错;保持 env 不变,交给 Rust 端报。
        pass


def ocr_image(data: bytes) -> list[dict[str, object]]:
    """对图片字节做本地 OCR,返回 ``[{text, bbox, confidence}, ...]``。

    在委托给 Rust ``_core.ocr_image`` 前,先把引擎指向 wheel 内自带的 PP-OCRv5 权重
    (见 :func:`_ensure_ocr_models_env`),使裸 ``pip install pptspine`` 即可离线全功能
    OCR。非图片字节会抛出类型化的 :class:`PptOcrError`(:class:`PptError` 子类),绝不
    panic。
    """
    _ensure_ocr_models_env()
    return _core_ocr_image(data)


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
