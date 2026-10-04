"""``pptspine._core`` —— Rust 扩展模块的类型存根(PEP 561)。

形状 / OCR 结果以 ``dict[str, Any]`` 返回(可自省);键见 README。
图表占位(``kind == "placeholder"``)的 ``chart`` 键形如 :class:`ChartDict`(非图表为 ``None``)。
"""

from __future__ import annotations

from os import PathLike
from typing import Any, Literal, TypedDict

__version__: str

class ChartSeriesDict(TypedDict):
    name: str | None
    values: list[float | None]
    format_code: str | None
    color: str | None
    point_colors: dict[int, str | None]
    labels: ChartLabelsDict | None

class ChartLabelsDict(TypedDict):
    show_val: bool
    show_cat_name: bool
    show_percent: bool

class ChartDict(TypedDict):
    kind: str
    title: str | None
    categories: list[str]
    series: list[ChartSeriesDict]
    bar_dir: str | None
    grouping: str | None
    three_d: bool
    combo: bool
    of_pie: bool
    warnings: list[str]

class PptError(Exception):
    """pptspine 异常层级的根。"""

class PptZipError(PptError): ...
class PptXmlError(PptError): ...
class PptUnsupportedError(PptError): ...
class PptOcrError(PptError): ...

class CommentDict(TypedDict):
    author: str | None
    initials: str | None
    datetime: str | None
    text: str | None
    position: tuple[int, int] | None
    replies: list[CommentDict]

class Slide:
    """一张幻灯片句柄。"""

    @property
    def index(self) -> int: ...
    @property
    def layout_name(self) -> str | None: ...
    @property
    def master_name(self) -> str | None: ...
    @property
    def text(self) -> str: ...
    @property
    def notes(self) -> str | None: ...
    @property
    def hidden(self) -> bool: ...
    def shapes(self) -> list[dict[str, Any]]: ...
    def comments(self) -> list[CommentDict]: ...

class Presentation:
    """一份已解析的演示文稿句柄。"""

    @property
    def slide_count(self) -> int: ...
    @property
    def slide_size(self) -> tuple[int, int]: ...
    @property
    def slide_size_points(self) -> tuple[float, float]: ...
    def slides(self) -> list[Slide]: ...
    def slide(self, index: int) -> Slide: ...
    def media_names(self) -> list[str]: ...
    def image_bytes(self, media_name: str) -> bytes | None: ...
    def to_text(
        self,
        *,
        order: Literal["visual", "document"] = "visual",
        include_hidden: bool = False,
        max_output_bytes: int | None = None,
    ) -> str: ...
    def to_markdown(
        self,
        *,
        order: Literal["visual", "document"] = "visual",
        include_hidden: bool = False,
        max_output_bytes: int | None = None,
    ) -> str: ...
    def sections(self) -> list[tuple[str, list[int]]]: ...
    def diagnostics(self) -> list[dict[str, str | int]]: ...
    @property
    def truncated(self) -> bool: ...
    def parse_report(self) -> dict[str, Any]: ...
    def core_properties(self) -> dict[str, str | None]: ...
    def to_pdf(
        self,
        *,
        font_map: dict[str, str] | None = None,
        include_hidden: bool = False,
        max_page_ops: int | None = None,
        max_total_ops: int | None = None,
    ) -> bytes: ...
    def save_pdf(
        self,
        path: str | PathLike[str],
        *,
        font_map: dict[str, str] | None = None,
        include_hidden: bool = False,
        max_page_ops: int | None = None,
        max_total_ops: int | None = None,
    ) -> None: ...
    def __len__(self) -> int: ...

def version() -> str: ...
def open(
    path: str | PathLike[str],
    *,
    max_entries: int | None = None,
    max_entry_bytes: int | None = None,
    max_total_bytes: int | None = None,
    max_compression_ratio: int | None = None,
    max_name_len: int | None = None,
    max_slides: int | None = None,
    max_diagram_shapes: int | None = None,
    max_diagram_text_bytes: int | None = None,
    max_chart_points: int | None = None,
    max_comments: int | None = None,
    max_part_shapes: int | None = None,
    max_total_shapes: int | None = None,
    max_part_items: int | None = None,
    max_total_items: int | None = None,
    max_model_bytes: int | None = None,
) -> Presentation:
    """解析 ``.pptx``。可选关键字参数调整 zip 解压限额(正整数,``None`` = 缺省:条目数 10000、
    单条目 256 MiB、总解压 1 GiB、压缩比 10000、条目名 1024 字节、幻灯片数 5000);非法值
    抛 ``ValueError``,超限抛 ``PptZipError``。另有三项"展开预算"(整个文档跨 frame 累计):
    ``max_diagram_shapes``(SmartArt 展开形状总数,缺省 100000)、``max_diagram_text_bytes``
    (SmartArt 文字总字节,缺省 8 MiB)、``max_chart_points``(图表数据点总数,缺省 1000000);
    超出时对应 frame 降级为占位框并记 ``smartart-degraded`` / ``chart-degraded`` 诊断,不抛错。
    ``max_comments``(批注总条数含回复,缺省 100000)超出时截断并记 ``comments-truncated`` 诊断。"""

def open_bytes(
    data: bytes,
    *,
    max_entries: int | None = None,
    max_entry_bytes: int | None = None,
    max_total_bytes: int | None = None,
    max_compression_ratio: int | None = None,
    max_name_len: int | None = None,
    max_slides: int | None = None,
    max_diagram_shapes: int | None = None,
    max_diagram_text_bytes: int | None = None,
    max_chart_points: int | None = None,
    max_comments: int | None = None,
    max_part_shapes: int | None = None,
    max_total_shapes: int | None = None,
    max_part_items: int | None = None,
    max_total_items: int | None = None,
    max_model_bytes: int | None = None,
) -> Presentation:
    """同 :func:`open`,输入为内存字节。"""
def ocr_image(data: bytes) -> list[dict[str, Any]]: ...
def reconstruct_image_table(data: bytes) -> list[dict[str, Any]]: ...
