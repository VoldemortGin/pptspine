"""测试夹具:用纯 Python ``zipfile`` 合成最小但合法的 ``.pptx``,**不落二进制 fixture**。

一个 ``.pptx`` 是 OOXML —— 一个装着 XML 部件的 zip 包。这里手写最小部件集合:
``[Content_Types].xml`` + 根关系 + ``presentation.xml`` 及其关系 + 一张 slide(含一个
文本框、一张两行两列表格)+ slide 关系。足够覆盖解析层的文本 / 表格 / 画布尺寸路径。
"""

from __future__ import annotations

import io
import zipfile
from pathlib import Path

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


# --- 进阶合成 fixture:内嵌图片 / 演讲者备注 / 合并单元格表格 ---------------------

_FIXTURES_DIR = Path(__file__).resolve().parent / "fixtures"

# 复用本仓已 vendored、已验证的 OCR 样张,作为内嵌图片字节(含已知参考行)。
_OCR_SAMPLE_PNG = _FIXTURES_DIR / "ocr_sample.png"


def _zip_pptx(parts: dict[str, bytes | str]) -> bytes:
    """把一组部件(路径 -> 文本/字节)打成内存里的 ``.pptx`` zip 字节串。"""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        for name, data in parts.items():
            z.writestr(name, data)
    return buf.getvalue()


# 一张内嵌图片的 slide(``p:pic`` 经 rels 关联到 ``ppt/media/image1.png``)。
_CONTENT_TYPES_IMAGE = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"""

_SLIDE_IMAGE = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:pic>
        <p:nvPicPr>
          <p:cNvPr id="2" name="Picture 1"/>
          <p:cNvPicPr/>
          <p:nvPr/>
        </p:nvPicPr>
        <p:blipFill><a:blip r:embed="rId1"/></p:blipFill>
        <p:spPr>
          <a:xfrm><a:off x="838200" y="365125"/><a:ext cx="2743200" cy="1143000"/></a:xfrm>
          <a:prstGeom prst="rect"/>
        </p:spPr>
      </p:pic>
    </p:spTree>
  </p:cSld>
</p:sld>"""

_SLIDE_IMAGE_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
</Relationships>"""


@pytest.fixture(scope="session")
def image_pptx() -> tuple[bytes, str, bytes]:
    """一张含内嵌图片的合成 ``.pptx``。返回 ``(pptx_bytes, media_name, png_bytes)``。"""
    png = _OCR_SAMPLE_PNG.read_bytes()
    pptx = _zip_pptx(
        {
            "[Content_Types].xml": _CONTENT_TYPES_IMAGE,
            "_rels/.rels": _ROOT_RELS,
            "ppt/presentation.xml": _PRESENTATION,
            "ppt/_rels/presentation.xml.rels": _PRESENTATION_RELS,
            "ppt/slides/slide1.xml": _SLIDE_IMAGE,
            "ppt/slides/_rels/slide1.xml.rels": _SLIDE_IMAGE_RELS,
            "ppt/media/image1.png": png,
        }
    )
    return pptx, "image1.png", png


# 一张带演讲者备注的 slide(经 rels 关联到 ``ppt/notesSlides/notesSlide1.xml``)。
_NOTES_TEXT = "Remember to smile\nSecond note line"

_CONTENT_TYPES_NOTES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
  <Override PartName="/ppt/notesSlides/notesSlide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml"/>
</Types>"""

_SLIDE_NOTES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:txBody>
          <a:p><a:r><a:t>Slide with notes</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>"""

_SLIDE_NOTES_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notesSlide1.xml"/>
</Relationships>"""

# 备注页:body 占位符里两段备注 + 一个非 body 占位符(应被忽略)。
_NOTES_SLIDE = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:notes xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
         xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
         xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:nvSpPr>
          <p:cNvPr id="2" name="Slide Image Placeholder 1"/>
          <p:cNvSpPr/>
          <p:nvPr><p:ph type="sldImg"/></p:nvPr>
        </p:nvSpPr>
        <p:txBody><a:p><a:r><a:t>NOT THE NOTES</a:t></a:r></a:p></p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr>
          <p:cNvPr id="3" name="Notes Placeholder 2"/>
          <p:cNvSpPr/>
          <p:nvPr><p:ph type="body" idx="1"/></p:nvPr>
        </p:nvSpPr>
        <p:txBody>
          <a:p><a:r><a:t>Remember to smile</a:t></a:r></a:p>
          <a:p><a:r><a:t>Second note line</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:notes>"""


@pytest.fixture(scope="session")
def notes_pptx() -> tuple[bytes, str]:
    """一张带演讲者备注的合成 ``.pptx``。返回 ``(pptx_bytes, expected_notes_text)``。"""
    pptx = _zip_pptx(
        {
            "[Content_Types].xml": _CONTENT_TYPES_NOTES,
            "_rels/.rels": _ROOT_RELS,
            "ppt/presentation.xml": _PRESENTATION,
            "ppt/_rels/presentation.xml.rels": _PRESENTATION_RELS,
            "ppt/slides/slide1.xml": _SLIDE_NOTES,
            "ppt/slides/_rels/slide1.xml.rels": _SLIDE_NOTES_RELS,
            "ppt/notesSlides/notesSlide1.xml": _NOTES_SLIDE,
        }
    )
    return pptx, _NOTES_TEXT


# 一张含合并单元格(gridSpan)的表格 slide,用于 to_markdown 的 HTML <table> 保真。
_SLIDE_MERGED = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:txBody><a:p><a:r><a:t>Merged Demo</a:t></a:r></a:p></p:txBody>
      </p:sp>
      <p:graphicFrame>
        <p:xfrm><a:off x="838200" y="2000250"/><a:ext cx="7772400" cy="2000250"/></p:xfrm>
        <a:graphic>
          <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
            <a:tbl>
              <a:tr h="370840">
                <a:tc gridSpan="2">
                  <a:txBody><a:p><a:r><a:t>Header</a:t></a:r></a:p></a:txBody>
                </a:tc>
                <a:tc hMerge="1">
                  <a:txBody><a:p/></a:txBody>
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


@pytest.fixture(scope="session")
def merged_table_pptx_bytes() -> bytes:
    """一张含 gridSpan 合并单元格表格的合成 ``.pptx`` 字节串。"""
    return _zip_pptx(
        {
            "[Content_Types].xml": _CONTENT_TYPES,
            "_rels/.rels": _ROOT_RELS,
            "ppt/presentation.xml": _PRESENTATION,
            "ppt/_rels/presentation.xml.rels": _PRESENTATION_RELS,
            "ppt/slides/slide1.xml": _SLIDE_MERGED,
            "ppt/slides/_rels/slide1.xml.rels": _ROOT_RELS_EMPTY,
        }
    )


# --- B-3 解析止损批 fixture(PRD-PDF-EXPORT §3.h/i/l/p/s/t/u)---------------------
#
# 一张覆盖全部止损点的 slide:段内换行 ``a:br`` + 字段 ``a:fld``(此前默默丢文本)、
# ``mc:AlternateContent``(此前整块跳过,现降入 Fallback)、连接线 ``p:cxnSp``(此前
# 被丢)、非表格 ``p:graphicFrame``(图表——此前连矩形一起消失,现保占位)、
# ``a:tblGrid`` 列宽、``a:ea``/``a:cs`` 字体 + 下划线/删除线。

_SLIDE_B3 = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:txBody>
          <a:p>
            <a:r><a:t>Line one</a:t></a:r>
            <a:br/>
            <a:r>
              <a:rPr u="sng" strike="sngStrike">
                <a:latin typeface="Calibri"/>
                <a:ea typeface="SimSun"/>
                <a:cs typeface="Arial"/>
              </a:rPr>
              <a:t>Line two</a:t>
            </a:r>
          </a:p>
          <a:p>
            <a:fld id="{93A18523-9C96-4A83-A5F6-000000000000}" type="slidenum">
              <a:t>1</a:t>
            </a:fld>
          </a:p>
        </p:txBody>
      </p:sp>
      <mc:AlternateContent>
        <mc:Choice Requires="cx1">
          <p:sp><p:txBody><a:p><a:r><a:t>NEWER CHOICE</a:t></a:r></a:p></p:txBody></p:sp>
        </mc:Choice>
        <mc:Fallback>
          <p:sp><p:txBody><a:p><a:r><a:t>Fallback shape</a:t></a:r></a:p></p:txBody></p:sp>
        </mc:Fallback>
      </mc:AlternateContent>
      <p:cxnSp>
        <p:nvCxnSpPr><p:cNvPr id="4" name="Connector 3"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr>
        <p:spPr>
          <a:xfrm><a:off x="100" y="200"/><a:ext cx="300" cy="400"/></a:xfrm>
          <a:prstGeom prst="straightConnector1"/>
          <a:ln w="19050">
            <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
            <a:prstDash val="dash"/>
          </a:ln>
        </p:spPr>
      </p:cxnSp>
      <p:graphicFrame>
        <p:xfrm><a:off x="1000" y="2000"/><a:ext cx="3000" cy="4000"/></p:xfrm>
        <a:graphic>
          <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">
            <c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId9"/>
          </a:graphicData>
        </a:graphic>
      </p:graphicFrame>
      <p:graphicFrame>
        <p:xfrm><a:off x="838200" y="2000250"/><a:ext cx="5120767" cy="741680"/></p:xfrm>
        <a:graphic>
          <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
            <a:tbl>
              <a:tblGrid>
                <a:gridCol w="3886200"/>
                <a:gridCol w="1234567"/>
              </a:tblGrid>
              <a:tr h="370840">
                <a:tc><a:txBody><a:p><a:r><a:t>A1</a:t></a:r></a:p></a:txBody></a:tc>
                <a:tc><a:txBody><a:p><a:r><a:t>B1</a:t></a:r></a:p></a:txBody></a:tc>
              </a:tr>
            </a:tbl>
          </a:graphicData>
        </a:graphic>
      </p:graphicFrame>
    </p:spTree>
  </p:cSld>
</p:sld>"""


@pytest.fixture(scope="session")
def b3_pptx_bytes() -> bytes:
    """覆盖 B-3 全部解析止损点的合成 ``.pptx`` 字节串(单 slide)。"""
    return _zip_pptx(
        {
            "[Content_Types].xml": _CONTENT_TYPES,
            "_rels/.rels": _ROOT_RELS,
            "ppt/presentation.xml": _PRESENTATION,
            "ppt/_rels/presentation.xml.rels": _PRESENTATION_RELS,
            "ppt/slides/slide1.xml": _SLIDE_B3,
            "ppt/slides/_rels/slide1.xml.rels": _ROOT_RELS_EMPTY,
        }
    )


# --- PDF 导出 fixture(PRD-PDF-EXPORT §8 B-1 / B-2 绿条)---------------------------
#
# B-1:16:9 双 slide 空文本框 deck(页数 / 页面尺寸门);B-2:显式几何单文本框 deck
# (今日五 run 属性:text / font / size_pt / bold / italic / color)。
# B-2 字号刻意 ≤ 20 pt 且用 Arial(替换 → Liberation Sans,hhea lineGap 67/2048):
# 首行文字顶 = 内容顶 + lineGap×size,20 pt 时 ≈ 0.65 pt,落在 1 pt 门限内。

# 16:9 画布(12192000×6858000 EMU = 960×540 pt)。
_SLIDE_CX_169 = 12_192_000
_SLIDE_CY_169 = 6_858_000

_PRESENTATION_169 = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
                xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
                xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldIdLst>
    <p:sldId id="256" r:id="rId1"/>
    <p:sldId id="257" r:id="rId2"/>
  </p:sldIdLst>
  <p:sldSz cx="{_SLIDE_CX_169}" cy="{_SLIDE_CY_169}" type="screen16x9"/>
</p:presentation>"""

_PRESENTATION_RELS_169 = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide2.xml"/>
</Relationships>"""

_CONTENT_TYPES_169 = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
  <Override PartName="/ppt/slides/slide2.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"""


def _simple_text_slide(text: str) -> str:
    return f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:spPr>
          <a:xfrm><a:off x="914400" y="914400"/><a:ext cx="4572000" cy="914400"/></a:xfrm>
        </p:spPr>
        <p:txBody>
          <a:p><a:r><a:rPr sz="2000"><a:latin typeface="Arial"/></a:rPr><a:t>{text}</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>"""


@pytest.fixture(scope="session")
def widescreen_pptx_bytes() -> bytes:
    """16:9(960×540 pt)双 slide 的合成 ``.pptx``(B-1 页数 / 页面尺寸门)。"""
    return _zip_pptx(
        {
            "[Content_Types].xml": _CONTENT_TYPES_169,
            "_rels/.rels": _ROOT_RELS,
            "ppt/presentation.xml": _PRESENTATION_169,
            "ppt/_rels/presentation.xml.rels": _PRESENTATION_RELS_169,
            "ppt/slides/slide1.xml": _simple_text_slide("First slide"),
            "ppt/slides/slide2.xml": _simple_text_slide("Second slide"),
            "ppt/slides/_rels/slide1.xml.rels": _ROOT_RELS_EMPTY,
            "ppt/slides/_rels/slide2.xml.rels": _ROOT_RELS_EMPTY,
        }
    )


# B-2 单文本框:显式矩形 off(914400,914400)=72,72 pt、ext(4572000,1828800)=360,144 pt。
_B2_RECT_EMU = (914_400, 914_400, 4_572_000, 1_828_800)

_SLIDE_B2 = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:spPr>
          <a:xfrm><a:off x="914400" y="914400"/><a:ext cx="4572000" cy="1828800"/></a:xfrm>
        </p:spPr>
        <p:txBody>
          <a:p>
            <a:r>
              <a:rPr sz="2000" b="1" i="0">
                <a:solidFill><a:srgbClr val="1F4E79"/></a:solidFill>
                <a:latin typeface="Arial"/>
              </a:rPr>
              <a:t>Alpha beta gamma</a:t>
            </a:r>
          </a:p>
          <a:p>
            <a:r>
              <a:rPr sz="1800" i="1">
                <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
                <a:latin typeface="Arial"/>
              </a:rPr>
              <a:t>delta epsilon</a:t>
            </a:r>
          </a:p>
        </p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>"""


@pytest.fixture(scope="session")
def b2_textbox_pptx() -> tuple[bytes, tuple[int, int, int, int]]:
    """B-2 单文本框 deck。返回 ``(pptx_bytes, rect_emu)``(4:3 画布)。"""
    pptx = _zip_pptx(
        {
            "[Content_Types].xml": _CONTENT_TYPES,
            "_rels/.rels": _ROOT_RELS,
            "ppt/presentation.xml": _PRESENTATION,
            "ppt/_rels/presentation.xml.rels": _PRESENTATION_RELS,
            "ppt/slides/slide1.xml": _SLIDE_B2,
            "ppt/slides/_rels/slide1.xml.rels": _ROOT_RELS_EMPTY,
        }
    )
    return pptx, _B2_RECT_EMU


# 两个子集外预设(cloud / heart)——告警**逐种类**上浮一次的门(PRD §6)。
_SLIDE_UNKNOWN_PRESETS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:spPr>
          <a:xfrm><a:off x="914400" y="914400"/><a:ext cx="1828800" cy="914400"/></a:xfrm>
          <a:prstGeom prst="cloud"/>
          <a:solidFill><a:srgbClr val="FFCC00"/></a:solidFill>
        </p:spPr>
      </p:sp>
      <p:sp>
        <p:spPr>
          <a:xfrm><a:off x="3657600" y="914400"/><a:ext cx="1828800" cy="914400"/></a:xfrm>
          <a:prstGeom prst="heart"/>
          <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
        </p:spPr>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>"""


@pytest.fixture(scope="session")
def unknown_presets_pptx_bytes() -> bytes:
    """两个 v1 子集外预设形状的合成 ``.pptx``(告警逐种类上浮门)。"""
    return _zip_pptx(
        {
            "[Content_Types].xml": _CONTENT_TYPES,
            "_rels/.rels": _ROOT_RELS,
            "ppt/presentation.xml": _PRESENTATION,
            "ppt/_rels/presentation.xml.rels": _PRESENTATION_RELS,
            "ppt/slides/slide1.xml": _SLIDE_UNKNOWN_PRESETS,
            "ppt/slides/_rels/slide1.xml.rels": _ROOT_RELS_EMPTY,
        }
    )


# --- 端到端综合 deck:完整继承链(layout/master/theme)+ 标题占位符 + 正文列表
# --- + 预设形状 + 图片(镜像 crates/ppt-parse/tests/resolve.rs 的链 fixture)。

_E2E_THEME = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office">
  <a:themeElements>
    <a:clrScheme name="Office">
      <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
      <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
      <a:dk2><a:srgbClr val="44546A"/></a:dk2>
      <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
      <a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
      <a:accent3><a:srgbClr val="A5A5A5"/></a:accent3>
      <a:accent4><a:srgbClr val="FFC000"/></a:accent4>
      <a:accent5><a:srgbClr val="5B9BD5"/></a:accent5>
      <a:accent6><a:srgbClr val="70AD47"/></a:accent6>
      <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
      <a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
    </a:clrScheme>
    <a:fontScheme name="Office">
      <a:majorFont><a:latin typeface="Arial"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont>
      <a:minorFont><a:latin typeface="Arial"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont>
    </a:fontScheme>
    <a:fmtScheme name="Office">
      <a:fillStyleLst>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
        <a:solidFill><a:schemeClr val="phClr"><a:tint val="40000"/></a:schemeClr></a:solidFill>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
      </a:fillStyleLst>
      <a:lnStyleLst>
        <a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>
        <a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>
        <a:ln w="19050"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>
      </a:lnStyleLst>
      <a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>
      <a:bgFillStyleLst>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
      </a:bgFillStyleLst>
    </a:fmtScheme>
  </a:themeElements>
</a:theme>"""

_E2E_MASTER = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
             xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
             xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Title Placeholder 1"/><p:cNvSpPr/>
          <p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="838200" y="365125"/><a:ext cx="7772400" cy="1325563"/></a:xfrm></p:spPr>
        <p:txBody><a:bodyPr/><a:lstStyle/>
          <a:p><a:r><a:t>Click to edit Master title style</a:t></a:r></a:p></p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Body Placeholder 2"/><p:cNvSpPr/>
          <p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
        <p:spPr><a:xfrm><a:off x="838200" y="1825625"/><a:ext cx="7772400" cy="3000000"/></a:xfrm></p:spPr>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>master body prompt</a:t></a:r></a:p></p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
  <p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2"
            accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6"
            hlink="hlink" folHlink="folHlink"/>
  <p:txStyles>
    <p:titleStyle>
      <a:lvl1pPr algn="ctr"><a:buNone/>
        <a:defRPr sz="3600"><a:solidFill><a:schemeClr val="tx2"/></a:solidFill><a:latin typeface="+mj-lt"/></a:defRPr>
      </a:lvl1pPr>
    </p:titleStyle>
    <p:bodyStyle>
      <a:lvl1pPr marL="342900" indent="-342900"><a:buFont typeface="Arial"/><a:buChar char="&#8226;"/>
        <a:defRPr sz="2000"><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl1pPr>
      <a:lvl2pPr marL="742950" indent="-285750"><a:buFont typeface="Arial"/><a:buChar char="&#8211;"/>
        <a:defRPr sz="1800"><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl2pPr>
    </p:bodyStyle>
    <p:otherStyle>
      <a:lvl1pPr><a:defRPr sz="1800"/></a:lvl1pPr>
    </p:otherStyle>
  </p:txStyles>
</p:sldMaster>"""

_E2E_LAYOUT = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
             xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
             xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/>
          <p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>layout title prompt</a:t></a:r></a:p></p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Content 2"/><p:cNvSpPr/>
          <p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/><a:lstStyle/><a:p/></p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
  <p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
</p:sldLayout>"""

# slide:标题占位符(无 xfrm / 无 rPr,几何与样式全走链)+ 正文列表(lvl 0/0/1)
# + roundRect 预设形状(accent1 填充 + 文字)+ 图片。
_E2E_SLIDE = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/>
          <p:nvPr><p:ph type="ctrTitle"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/><a:p><a:r><a:t>Quarterly Review</a:t></a:r></a:p></p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Content 2"/><p:cNvSpPr/>
          <p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr>
        <p:spPr/>
        <p:txBody><a:bodyPr/>
          <a:p><a:r><a:t>Revenue up strongly</a:t></a:r></a:p>
          <a:p><a:r><a:t>Costs held flat</a:t></a:r></a:p>
          <a:p><a:pPr lvl="1"/><a:r><a:t>Cloud spend detail</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
      <p:sp>
        <p:spPr>
          <a:xfrm><a:off x="838200" y="5100000"/><a:ext cx="2400000" cy="1000000"/></a:xfrm>
          <a:prstGeom prst="roundRect"/>
          <a:solidFill><a:schemeClr val="accent1"/></a:solidFill>
          <a:ln w="19050"><a:solidFill><a:srgbClr val="2F5597"/></a:solidFill></a:ln>
        </p:spPr>
        <p:txBody>
          <a:p><a:pPr algn="ctr"/>
            <a:r><a:rPr sz="1800" b="1"><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill>
              <a:latin typeface="Arial"/></a:rPr><a:t>GO TEAM</a:t></a:r>
          </a:p>
        </p:txBody>
      </p:sp>
      <p:pic>
        <p:nvPicPr><p:cNvPr id="5" name="Picture 4"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
        <p:blipFill><a:blip r:embed="rId2"/></p:blipFill>
        <p:spPr>
          <a:xfrm><a:off x="5486400" y="5100000"/><a:ext cx="2743200" cy="1143000"/></a:xfrm>
          <a:prstGeom prst="rect"/>
        </p:spPr>
      </p:pic>
    </p:spTree>
  </p:cSld>
</p:sld>"""

_E2E_CONTENT_TYPES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
</Types>"""

_E2E_SLIDE_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
</Relationships>"""

_E2E_LAYOUT_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/>
</Relationships>"""

_E2E_MASTER_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/>
</Relationships>"""


@pytest.fixture(scope="session")
def e2e_pptx_bytes() -> bytes:
    """端到端综合 deck:完整继承链 + 标题占位符 + 正文列表 + 预设形状 + 图片。"""
    return build_e2e_pptx()


def build_e2e_pptx() -> bytes:
    """构造端到端综合 deck 字节(独立函数,供脚本 / 光栅目检复用)。"""
    png = _OCR_SAMPLE_PNG.read_bytes()
    return _zip_pptx(
        {
            "[Content_Types].xml": _E2E_CONTENT_TYPES,
            "_rels/.rels": _ROOT_RELS,
            "ppt/presentation.xml": _PRESENTATION,
            "ppt/_rels/presentation.xml.rels": _PRESENTATION_RELS,
            "ppt/slides/slide1.xml": _E2E_SLIDE,
            "ppt/slides/_rels/slide1.xml.rels": _E2E_SLIDE_RELS,
            "ppt/slideLayouts/slideLayout1.xml": _E2E_LAYOUT,
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels": _E2E_LAYOUT_RELS,
            "ppt/slideMasters/slideMaster1.xml": _E2E_MASTER,
            "ppt/slideMasters/_rels/slideMaster1.xml.rels": _E2E_MASTER_RELS,
            "ppt/theme/theme1.xml": _E2E_THEME,
            "ppt/media/image1.png": png,
        }
    )


# --- B-10 背景继承(slide 无 bg → layout → master)端到端 fixture ----------------

_MASTER_BG_SLIDE = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree/></p:cSld>
</p:sld>"""

_MASTER_BG_SLIDE_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
</Relationships>"""

_MASTER_BG_LAYOUT = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
             xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
             xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree/></p:cSld>
</p:sldLayout>"""

_MASTER_BG_LAYOUT_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/>
</Relationships>"""

# 纯绿背景(00FF00),留待端到端断言其满页填充(clean 0/1 分量,内容流可直接 grep)。
_MASTER_BG_MASTER = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
             xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
             xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:bg><p:bgPr><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></p:bgPr></p:bg>
    <p:spTree/>
  </p:cSld>
</p:sldMaster>"""


@pytest.fixture(scope="session")
def master_background_pptx_bytes() -> bytes:
    """slide / layout 均无 `p:bg`,slideMaster 带纯色背景(B-10 layout/master 继承门)。"""
    return _zip_pptx(
        {
            "[Content_Types].xml": _CONTENT_TYPES,
            "_rels/.rels": _ROOT_RELS,
            "ppt/presentation.xml": _PRESENTATION,
            "ppt/_rels/presentation.xml.rels": _PRESENTATION_RELS,
            "ppt/slides/slide1.xml": _MASTER_BG_SLIDE,
            "ppt/slides/_rels/slide1.xml.rels": _MASTER_BG_SLIDE_RELS,
            "ppt/slideLayouts/slideLayout1.xml": _MASTER_BG_LAYOUT,
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels": _MASTER_BG_LAYOUT_RELS,
            "ppt/slideMasters/slideMaster1.xml": _MASTER_BG_MASTER,
        }
    )


# --- B-4 / B-5 fixture(PRD-PDF-EXPORT §8:形状变换 / Group 仿射)-----------------
#
# B-4:旋转 45° 文本框(词心 1 pt 门)、roundRect avLst 调整(光栅对照)、
# rtTriangle flipH(光栅非对称)、prstDash 虚线(op 断言)、srcRect 裁剪(光栅对照)。
# B-5:组合缩放文本框与"拆组等价"孪生 deck(get_text_words 1 pt 门)+ 嵌套组合。


def _sp_tree_slide(sp_tree_inner: str) -> str:
    return f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>{sp_tree_inner}</p:spTree></p:cSld>
</p:sld>"""


def build_sp_tree_pptx(sp_tree_inner: str) -> bytes:
    """给定 ``p:spTree`` 内容,合成单 slide 4:3 deck(B-4/B-5 fixture 共用)。"""
    return _zip_pptx(
        {
            "[Content_Types].xml": _CONTENT_TYPES,
            "_rels/.rels": _ROOT_RELS,
            "ppt/presentation.xml": _PRESENTATION,
            "ppt/_rels/presentation.xml.rels": _PRESENTATION_RELS,
            "ppt/slides/slide1.xml": _sp_tree_slide(sp_tree_inner),
            "ppt/slides/_rels/slide1.xml.rels": _ROOT_RELS_EMPTY,
        }
    )


# 旋转文本框:单行 "Center" 居中对齐;盒高按 Liberation Sans 度量调到
# "词心 = 矩形中心"(hhea lineGap 67 / ascent 1854 / descent 434,em 2048;
# 词心离内容顶 ≈ 11.5–11.8 pt,取中值 11.66 → 盒高 2×(3.6+11.66) ≈ 30.52 pt)。
_ROT_RECT_EMU = (914_400, 1_828_800, 2_286_000, 387_604)


def _rotated_textbox_slide(rot: int) -> str:
    x, y, w, h = _ROT_RECT_EMU
    return f"""<p:sp>
        <p:spPr>
          <a:xfrm rot="{rot}"><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></a:xfrm>
        </p:spPr>
        <p:txBody>
          <a:p><a:pPr algn="ctr"/>
            <a:r><a:rPr sz="2000"><a:latin typeface="Arial"/></a:rPr><a:t>Center</a:t></a:r>
          </a:p>
        </p:txBody>
      </p:sp>"""


@pytest.fixture(scope="session")
def rotated_textbox_pptx() -> tuple[bytes, bytes, tuple[int, int, int, int]]:
    """B-4 旋转门:``(rot45_pptx, plain_pptx, rect_emu)``。"""
    return (
        build_sp_tree_pptx(_rotated_textbox_slide(45 * 60_000)),
        build_sp_tree_pptx(_rotated_textbox_slide(0)),
        _ROT_RECT_EMU,
    )


def _round_rect_slide(av_lst: str) -> str:
    return f"""<p:sp>
        <p:spPr>
          <a:xfrm><a:off x="914400" y="914400"/><a:ext cx="2743200" cy="1828800"/></a:xfrm>
          <a:prstGeom prst="roundRect">{av_lst}</a:prstGeom>
          <a:solidFill><a:srgbClr val="CC0000"/></a:solidFill>
        </p:spPr>
      </p:sp>"""


@pytest.fixture(scope="session")
def round_rect_adjust_pptx() -> tuple[bytes, bytes]:
    """B-4 avLst 门:``(adjusted_pptx, default_pptx)``(adj=50000 vs 缺省)。"""
    adjusted = _round_rect_slide('<a:avLst><a:gd name="adj" fmla="val 50000"/></a:avLst>')
    default = _round_rect_slide("<a:avLst/>")
    return build_sp_tree_pptx(adjusted), build_sp_tree_pptx(default)


def _rt_triangle_slide(flip: str) -> str:
    return f"""<p:sp>
        <p:spPr>
          <a:xfrm{flip}><a:off x="914400" y="914400"/><a:ext cx="3657600" cy="2743200"/></a:xfrm>
          <a:prstGeom prst="rtTriangle"/>
          <a:solidFill><a:srgbClr val="000080"/></a:solidFill>
        </p:spPr>
      </p:sp>"""


@pytest.fixture(scope="session")
def flipped_triangle_pptx() -> tuple[bytes, bytes]:
    """B-4 翻转门:``(flipped_pptx, plain_pptx)``(rtTriangle 直角边随 flipH 换边)。"""
    return (
        build_sp_tree_pptx(_rt_triangle_slide(' flipH="1"')),
        build_sp_tree_pptx(_rt_triangle_slide("")),
    )


_DASH_CONNECTOR = """<p:cxnSp>
        <p:nvCxnSpPr><p:cNvPr id="4" name="Connector 3"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr>
        <p:spPr>
          <a:xfrm><a:off x="914400" y="914400"/><a:ext cx="3657600" cy="1828800"/></a:xfrm>
          <a:prstGeom prst="straightConnector1"/>
          <a:ln w="25400">
            <a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>
            <a:prstDash val="dash"/>
          </a:ln>
        </p:spPr>
      </p:cxnSp>"""


@pytest.fixture(scope="session")
def dashed_connector_pptx() -> bytes:
    """B-4 虚线门:prstDash="dash"、线宽 2 pt 的连接线。"""
    return build_sp_tree_pptx(_DASH_CONNECTOR)


def _picture_slide(blip_extra: str) -> str:
    return f"""<p:pic>
        <p:nvPicPr><p:cNvPr id="2" name="Picture 1"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
        <p:blipFill><a:blip r:embed="rId1"/>{blip_extra}</p:blipFill>
        <p:spPr>
          <a:xfrm><a:off x="914400" y="914400"/><a:ext cx="2743200" cy="1143000"/></a:xfrm>
          <a:prstGeom prst="rect"/>
        </p:spPr>
      </p:pic>"""


def _picture_pptx(blip_extra: str) -> bytes:
    png = _OCR_SAMPLE_PNG.read_bytes()
    return _zip_pptx(
        {
            "[Content_Types].xml": _CONTENT_TYPES_IMAGE,
            "_rels/.rels": _ROOT_RELS,
            "ppt/presentation.xml": _PRESENTATION,
            "ppt/_rels/presentation.xml.rels": _PRESENTATION_RELS,
            "ppt/slides/slide1.xml": _sp_tree_slide(_picture_slide(blip_extra)),
            "ppt/slides/_rels/slide1.xml.rels": _SLIDE_IMAGE_RELS,
            "ppt/media/image1.png": png,
        }
    )


@pytest.fixture(scope="session")
def src_rect_pptx() -> tuple[bytes, bytes]:
    """B-4 srcRect 门:``(cropped_pptx, plain_pptx)``(裁掉源图右半)。"""
    return (
        _picture_pptx('<a:srcRect r="50000"/>'),
        _picture_pptx(""),
    )


# B-5:组合缩放文本框(child (0,0,144,72)pt → rect (72,72,288,144)pt,s=2)与
# 拆组等价孪生(同字号——PowerPoint 语义:组合缩放不缩放字号)。
_GROUP_TEXT_BODY = """<p:txBody>
          <a:p><a:r><a:rPr sz="2000"><a:latin typeface="Arial"/></a:rPr><a:t>Twin gate</a:t></a:r></a:p>
        </p:txBody>"""

_GROUPED_SLIDE = f"""<p:grpSp>
        <p:nvGrpSpPr><p:cNvPr id="10" name="Group 9"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
        <p:grpSpPr>
          <a:xfrm>
            <a:off x="914400" y="914400"/><a:ext cx="3657600" cy="1828800"/>
            <a:chOff x="0" y="0"/><a:chExt cx="1828800" cy="914400"/>
          </a:xfrm>
        </p:grpSpPr>
        <p:sp>
          <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1828800" cy="914400"/></a:xfrm></p:spPr>
          {_GROUP_TEXT_BODY}
        </p:sp>
      </p:grpSp>"""

_GROUP_TWIN_SLIDE = f"""<p:sp>
        <p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="3657600" cy="1828800"/></a:xfrm></p:spPr>
        {_GROUP_TEXT_BODY}
      </p:sp>"""


@pytest.fixture(scope="session")
def grouped_textbox_pptx() -> tuple[bytes, bytes]:
    """B-5 孪生门:``(grouped_pptx, flattened_twin_pptx)``。"""
    return build_sp_tree_pptx(_GROUPED_SLIDE), build_sp_tree_pptx(_GROUP_TWIN_SLIDE)


# 嵌套组合:外层 (0,0,216,108)pt → (72,72,432,216)pt,内层 (0,0,72,36)pt →
# (36,18,144,72)pt;复合映射文本框 (0,0,72,36) → (144,108,288,144)pt。
_NESTED_TEXT_BODY = """<p:txBody>
          <a:p><a:r><a:rPr sz="2000"><a:latin typeface="Arial"/></a:rPr><a:t>Deep nest</a:t></a:r></a:p>
        </p:txBody>"""

_NESTED_SLIDE = f"""<p:grpSp>
        <p:nvGrpSpPr><p:cNvPr id="20" name="Outer"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
        <p:grpSpPr>
          <a:xfrm>
            <a:off x="914400" y="914400"/><a:ext cx="5486400" cy="2743200"/>
            <a:chOff x="0" y="0"/><a:chExt cx="2743200" cy="1371600"/>
          </a:xfrm>
        </p:grpSpPr>
        <p:grpSp>
          <p:nvGrpSpPr><p:cNvPr id="21" name="Inner"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
          <p:grpSpPr>
            <a:xfrm>
              <a:off x="457200" y="228600"/><a:ext cx="1828800" cy="914400"/>
              <a:chOff x="0" y="0"/><a:chExt cx="914400" cy="457200"/>
            </a:xfrm>
          </p:grpSpPr>
          <p:sp>
            <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/></a:xfrm></p:spPr>
            {_NESTED_TEXT_BODY}
          </p:sp>
        </p:grpSp>
      </p:grpSp>"""

_NESTED_TWIN_SLIDE = f"""<p:sp>
        <p:spPr><a:xfrm><a:off x="1828800" y="1371600"/><a:ext cx="3657600" cy="1828800"/></a:xfrm></p:spPr>
        {_NESTED_TEXT_BODY}
      </p:sp>"""


@pytest.fixture(scope="session")
def nested_group_pptx() -> tuple[bytes, bytes]:
    """B-5 嵌套门:``(nested_pptx, flattened_twin_pptx)``。"""
    return build_sp_tree_pptx(_NESTED_SLIDE), build_sp_tree_pptx(_NESTED_TWIN_SLIDE)


# --- B-6 fixture(PRD-PDF-EXPORT §8 B-6:锚定 / 行距 / 项目符号)-------------------
#
# B-6.1 底锚:``anchor="b"`` 单行文本框 + 同尺寸顶锚孪生(内容块底贴内容矩形底)。
# B-6.2 行距:同段两行(``a:br`` 硬换行),``lnSpc`` 100% 与 200% 两 deck(行距翻倍)。
# B-6.3 项目符号:``buChar`` '•' + ``buAutoNum`` arabicPeriod '1.' 读回可见。

# 底锚文本框:显式矩形 off(914400,914400)=72,72pt、ext(4572000,1828800)=360,144pt。
_B6_ANCHOR_RECT_EMU = (914_400, 914_400, 4_572_000, 1_828_800)


def _anchored_textbox_slide(anchor: str) -> str:
    x, y, w, h = _B6_ANCHOR_RECT_EMU
    # "Anchor" 无降部字形——但 get_text_words 的 bbox 仍取字体行盒(ascent→descent)。
    return f"""<p:sp>
        <p:spPr>
          <a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></a:xfrm>
        </p:spPr>
        <p:txBody>
          <a:bodyPr anchor="{anchor}"/>
          <a:p><a:r><a:rPr sz="2000"><a:latin typeface="Arial"/></a:rPr><a:t>Anchor</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>"""


@pytest.fixture(scope="session")
def anchored_textbox_pptx() -> tuple[bytes, bytes, tuple[int, int, int, int]]:
    """B-6 锚定门:``(bottom_anchor_pptx, top_anchor_pptx, rect_emu)``。"""
    return (
        build_sp_tree_pptx(_anchored_textbox_slide("b")),
        build_sp_tree_pptx(_anchored_textbox_slide("t")),
        _B6_ANCHOR_RECT_EMU,
    )


def _line_spacing_slide(pct: str) -> str:
    # 同一段落两行(a:br 硬换行);Alpha 在行 1、Beta 在行 2,便于逐词测 y。
    return f"""<p:sp>
        <p:spPr>
          <a:xfrm><a:off x="914400" y="914400"/><a:ext cx="4572000" cy="1828800"/></a:xfrm>
        </p:spPr>
        <p:txBody>
          <a:bodyPr/>
          <a:p>
            <a:pPr><a:lnSpc><a:spcPct val="{pct}"/></a:lnSpc></a:pPr>
            <a:r><a:rPr sz="2000"><a:latin typeface="Arial"/></a:rPr><a:t>Alpha</a:t></a:r>
            <a:br/>
            <a:r><a:rPr sz="2000"><a:latin typeface="Arial"/></a:rPr><a:t>Beta</a:t></a:r>
          </a:p>
        </p:txBody>
      </p:sp>"""


@pytest.fixture(scope="session")
def line_spacing_pptx() -> tuple[bytes, bytes]:
    """B-6 行距门:``(lnspc_100_pptx, lnspc_200_pptx)``(同段两行,a:br 硬换行)。"""
    return (
        build_sp_tree_pptx(_line_spacing_slide("100000")),
        build_sp_tree_pptx(_line_spacing_slide("200000")),
    )


# 正文文本框:段 1 buChar '•' + 段 2 buAutoNum arabicPeriod(首号 '1.')。
_BULLET_TEXTBOX = """<p:sp>
        <p:spPr>
          <a:xfrm><a:off x="914400" y="914400"/><a:ext cx="4572000" cy="1828800"/></a:xfrm>
        </p:spPr>
        <p:txBody>
          <a:bodyPr/>
          <a:p>
            <a:pPr marL="342900" indent="-342900"><a:buFont typeface="Arial"/><a:buChar char="&#8226;"/></a:pPr>
            <a:r><a:rPr sz="2000"><a:latin typeface="Arial"/></a:rPr><a:t>First item</a:t></a:r>
          </a:p>
          <a:p>
            <a:pPr marL="342900" indent="-342900"><a:buAutoNum type="arabicPeriod"/></a:pPr>
            <a:r><a:rPr sz="2000"><a:latin typeface="Arial"/></a:rPr><a:t>Second item</a:t></a:r>
          </a:p>
        </p:txBody>
      </p:sp>"""


@pytest.fixture(scope="session")
def bulleted_textbox_pptx_bytes() -> bytes:
    """B-6 项目符号门:buChar '•' + buAutoNum arabicPeriod '1.' 两段正文。"""
    return build_sp_tree_pptx(_BULLET_TEXTBOX)


# --- B-7 fixture(PRD-PDF-EXPORT §8 B-7:表格几何 / 边框 / 合并)-------------------
#
# 列宽之和 == 表 ext cx(消除比例缩放歧义):列边界 pt = off_x + 累积列宽 pt。
# B-7.1 每格首词 x;B-7.2 tcBorders 网格线可见;B-7.3 合并格内部无竖直边框。

_B7_TABLE_OFF = (914_400, 1_828_800)  # 72,144 pt
_B7_TABLE_CY = 741_680  # 58.4 pt(单行占满表高)
_B7_COL_WIDTHS = (2_286_000, 1_143_000, 1_143_000)  # 180,90,90 pt;和 = ext cx
_B7_CELL_TEXTS = ("Aaa", "Bbb", "Ccc")
_B7_MERGE_COLS = (2_286_000, 1_143_000)  # 180,90 pt(合并 / 反衬用)

# 四边红实线 tcBorders(w=12700=1pt、srgbClr FF0000)。
_B7_TC_BORDER = (
    "<a:tcPr>"
    '<a:lnL w="12700"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:lnL>'
    '<a:lnR w="12700"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:lnR>'
    '<a:lnT w="12700"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:lnT>'
    '<a:lnB w="12700"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:lnB>'
    "</a:tcPr>"
)


def _table_cell(text: str, border: str = "", extra_attr: str = "") -> str:
    body = (
        f'<a:txBody><a:p><a:r><a:rPr sz="1400"><a:latin typeface="Arial"/></a:rPr>'
        f"<a:t>{text}</a:t></a:r></a:p></a:txBody>"
        if text
        else "<a:txBody><a:p/></a:txBody>"
    )
    return f"<a:tc{extra_attr}>{body}{border}</a:tc>"


def _table_frame(
    off: tuple[int, int], ext_cx: int, ext_cy: int, col_widths: tuple[int, ...], row_inner: str
) -> str:
    ox, oy = off
    grid = "".join(f'<a:gridCol w="{w}"/>' for w in col_widths)
    return f"""<p:graphicFrame>
        <p:xfrm><a:off x="{ox}" y="{oy}"/><a:ext cx="{ext_cx}" cy="{ext_cy}"/></p:xfrm>
        <a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
          <a:tbl>
            <a:tblGrid>{grid}</a:tblGrid>
            <a:tr h="370840">{row_inner}</a:tr>
          </a:tbl>
        </a:graphicData></a:graphic>
      </p:graphicFrame>"""


@pytest.fixture(scope="session")
def grid_table_pptx() -> tuple[bytes, tuple[int, int], tuple[int, ...], int, tuple[str, ...]]:
    """B-7.1/B-7.2 表格门:``(pptx, off_emu, col_widths_emu, ext_cy_emu, cell_texts)``。

    单行三列,列宽之和 == 表 ext cx(免比例缩放),每格四边 tcBorders 红实线、首词可区分。
    """
    ext_cx = sum(_B7_COL_WIDTHS)
    row = "".join(_table_cell(t, _B7_TC_BORDER) for t in _B7_CELL_TEXTS)
    pptx = build_sp_tree_pptx(
        _table_frame(_B7_TABLE_OFF, ext_cx, _B7_TABLE_CY, _B7_COL_WIDTHS, row)
    )
    return pptx, _B7_TABLE_OFF, _B7_COL_WIDTHS, _B7_TABLE_CY, _B7_CELL_TEXTS


@pytest.fixture(scope="session")
def merged_border_table_pptx() -> tuple[bytes, bytes, tuple[int, int], tuple[int, ...], int]:
    """B-7.3 合并门:``(merged_pptx, plain_pptx, off_emu, col_widths_emu, ext_cy_emu)``。

    merged:一行 gridSpan=2 跨列格(+ 其后 hMerge 延续格),四边 tcBorders;
    plain:同尺寸不合并双列行(各格四边 tcBorders),反衬合并格内部无竖直分隔线。
    """
    ext_cx = sum(_B7_MERGE_COLS)
    merged_row = _table_cell("Merged", _B7_TC_BORDER, ' gridSpan="2"') + _table_cell(
        "", extra_attr=' hMerge="1"'
    )
    plain_row = _table_cell("Left", _B7_TC_BORDER) + _table_cell("Right", _B7_TC_BORDER)
    merged = build_sp_tree_pptx(
        _table_frame(_B7_TABLE_OFF, ext_cx, _B7_TABLE_CY, _B7_MERGE_COLS, merged_row)
    )
    plain = build_sp_tree_pptx(
        _table_frame(_B7_TABLE_OFF, ext_cx, _B7_TABLE_CY, _B7_MERGE_COLS, plain_row)
    )
    return merged, plain, _B7_TABLE_OFF, _B7_MERGE_COLS, _B7_TABLE_CY


# --- 纵排告警 fixture(Task 4 / PRD §6:bodyPr@vert 水平降级,告警逐种类一次)-----


def _vertical_textbox_slide(x: int, text: str) -> str:
    return f"""<p:sp>
        <p:spPr><a:xfrm><a:off x="{x}" y="914400"/><a:ext cx="1828800" cy="2743200"/></a:xfrm></p:spPr>
        <p:txBody>
          <a:bodyPr vert="vert270"/>
          <a:p><a:r><a:rPr sz="2000"><a:latin typeface="Arial"/></a:rPr><a:t>{text}</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>"""


@pytest.fixture(scope="session")
def vertical_text_pptx_bytes() -> bytes:
    """两个 ``bodyPr@vert=vert270`` 纵排文本框的合成 ``.pptx``(告警逐种类一次)。"""
    return build_sp_tree_pptx(
        _vertical_textbox_slide(914_400, "Vertical one")
        + _vertical_textbox_slide(3_657_600, "Vertical two")
    )
