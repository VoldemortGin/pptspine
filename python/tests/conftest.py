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
