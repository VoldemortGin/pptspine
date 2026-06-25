"""测试夹具:用纯 Python ``zipfile`` 合成最小但合法的 ``.pptx``,**不落二进制 fixture**。

一个 ``.pptx`` 是 OOXML —— 一个装着 XML 部件的 zip 包。这里手写最小部件集合:
``[Content_Types].xml`` + 根关系 + ``presentation.xml`` 及其关系 + 一张 slide(含一个
文本框、一张两行两列表格)+ slide 关系。足够覆盖解析层的文本 / 表格 / 画布尺寸路径。
"""

from __future__ import annotations

import io
import zipfile

import pytest

# 画布尺寸:标准 4:3,EMU(914400 EMU = 1 inch)。
_SLIDE_CX = 9_144_000
_SLIDE_CY = 6_858_000

_CONTENT_TYPES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"""

_ROOT_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>"""

_PRESENTATION = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
                xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldIdLst>
    <p:sldId id="256" r:id="rId1"/>
  </p:sldIdLst>
  <p:sldSz cx="{_SLIDE_CX}" cy="{_SLIDE_CY}" type="screen4x3"/>
</p:presentation>"""

_PRESENTATION_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>"""

# 一张 slide:一个带样式 run 的文本框 + 一张 2x2 表格。
_SLIDE1 = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:spPr>
          <a:xfrm>
            <a:off x="838200" y="365125"/>
            <a:ext cx="7772400" cy="1325563"/>
          </a:xfrm>
        </p:spPr>
        <p:txBody>
          <a:p>
            <a:pPr algn="ctr"/>
            <a:r>
              <a:rPr sz="4400" b="1" i="0">
                <a:solidFill><a:srgbClr val="1F4E79"/></a:solidFill>
                <a:latin typeface="Calibri"/>
              </a:rPr>
              <a:t>Hello pptspine</a:t>
            </a:r>
          </a:p>
          <a:p>
            <a:r>
              <a:rPr sz="2000"/>
              <a:t>second line</a:t>
            </a:r>
          </a:p>
        </p:txBody>
      </p:sp>
      <p:graphicFrame>
        <p:xfrm>
          <a:off x="838200" y="2000250"/>
          <a:ext cx="7772400" cy="2000250"/>
        </p:xfrm>
        <a:graphic>
          <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
            <a:tbl>
              <a:tr h="370840">
                <a:tc>
                  <a:txBody><a:p><a:r><a:t>A1</a:t></a:r></a:p></a:txBody>
                  <a:tcPr><a:solidFill><a:srgbClr val="FFCC00"/></a:solidFill></a:tcPr>
                </a:tc>
                <a:tc>
                  <a:txBody><a:p><a:r><a:t>B1</a:t></a:r></a:p></a:txBody>
                </a:tc>
              </a:tr>
              <a:tr h="370840">
                <a:tc>
                  <a:txBody><a:p><a:r><a:t>A2</a:t></a:r></a:p></a:txBody>
                </a:tc>
                <a:tc>
                  <a:txBody><a:p><a:r><a:t>B2</a:t></a:r></a:p></a:txBody>
                </a:tc>
              </a:tr>
            </a:tbl>
          </a:graphicData>
        </a:graphic>
      </p:graphicFrame>
    </p:spTree>
  </p:cSld>
</p:sld>"""


def _build_minimal_pptx() -> bytes:
    """把上面的部件打成一个内存里的 ``.pptx`` zip 字节串。"""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", _CONTENT_TYPES)
        z.writestr("_rels/.rels", _ROOT_RELS)
        z.writestr("ppt/presentation.xml", _PRESENTATION)
        z.writestr("ppt/_rels/presentation.xml.rels", _PRESENTATION_RELS)
        z.writestr("ppt/slides/slide1.xml", _SLIDE1)
        z.writestr("ppt/slides/_rels/slide1.xml.rels", _ROOT_RELS_EMPTY)
    return buf.getvalue()


_ROOT_RELS_EMPTY = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"""


@pytest.fixture(scope="session")
def minimal_pptx_bytes() -> bytes:
    """一个合成的最小 ``.pptx`` 字节串(一张 slide:文本框 + 2x2 表格)。"""
    return _build_minimal_pptx()


@pytest.fixture
def minimal_pptx_path(minimal_pptx_bytes: bytes, tmp_path) -> str:
    """把合成的 ``.pptx`` 落到临时文件,返回其路径(测 ``open(path)`` 路径)。"""
    p = tmp_path / "deck.pptx"
    p.write_bytes(minimal_pptx_bytes)
    return str(p)


@pytest.fixture(scope="session")
def slide_canvas() -> tuple[int, int]:
    """合成 deck 的画布尺寸 ``(cx, cy)``(EMU),供断言复用。"""
    return (_SLIDE_CX, _SLIDE_CY)
