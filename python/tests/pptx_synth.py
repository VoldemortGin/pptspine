"""测试用的最小 ``.pptx`` 现场合成器(纯 ``zipfile``,不落二进制 fixture)。

``build_pptx`` 接收若干 slide(``spTree`` 内容 + 关系 + ``p:sld`` 属性)与任意额外部件,
打成内存 zip;SmartArt / 批注 / OLE / 页脚等专项测试共用。
"""

from __future__ import annotations

import io
import zipfile
from dataclasses import dataclass, field

NS = (
    'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
    'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" '
    'xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"'
)
REL_NS = "http://schemas.openxmlformats.org/package/2006/relationships"
REL_BASE = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"


@dataclass
class SlideSpec:
    """一张 slide:``spTree`` 内部 XML、关系 ``(id, type, target)`` 列表、``p:sld`` 额外属性。"""

    sp_tree: str
    rels: list[tuple[str, str, str]] = field(default_factory=list)
    attrs: str = ""


def rels_xml(entries: list[tuple[str, str, str]]) -> str:
    body = "".join(f'<Relationship Id="{i}" Type="{t}" Target="{g}"/>' for i, t, g in entries)
    return f'<Relationships xmlns="{REL_NS}">{body}</Relationships>'


def build_pptx(
    slides: list[SlideSpec],
    parts: dict[str, str | bytes] | None = None,
    pres_attrs: str = "",
    size: tuple[int, int] = (9144000, 6858000),
) -> bytes:
    """合成 ``.pptx`` 字节串;``parts`` 是额外部件(路径 → 文本 / 字节)。"""
    ids = "".join(f'<p:sldId id="{256 + i}" r:id="rId{i + 1}"/>' for i in range(len(slides)))
    pres_rels = [(f"rId{i + 1}", f"{REL_BASE}/slide", f"slides/slide{i + 1}.xml") for i in range(len(slides))]
    files: dict[str, str | bytes] = {
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>',
        "_rels/.rels": rels_xml([("rId1", f"{REL_BASE}/officeDocument", "ppt/presentation.xml")]),
        "ppt/presentation.xml": (
            f"<p:presentation {NS}{pres_attrs}><p:sldIdLst>{ids}</p:sldIdLst>"
            f'<p:sldSz cx="{size[0]}" cy="{size[1]}"/></p:presentation>'
        ),
        "ppt/_rels/presentation.xml.rels": rels_xml(pres_rels),
    }
    for i, s in enumerate(slides, start=1):
        files[f"ppt/slides/slide{i}.xml"] = (
            f"<p:sld {NS}{s.attrs}><p:cSld><p:spTree>{s.sp_tree}</p:spTree></p:cSld></p:sld>"
        )
        files[f"ppt/slides/_rels/slide{i}.xml.rels"] = rels_xml(s.rels)
    files.update(parts or {})
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        for name, data in files.items():
            z.writestr(name, data)
    return buf.getvalue()
