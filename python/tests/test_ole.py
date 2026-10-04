"""OLE 对象的预览图:``p:oleObj > p:pic`` 作为图片进入 ``shapes()`` / Markdown / PDF
(直接写法与 ``mc:AlternateContent`` 包整个 graphicFrame 两种);EMF 预览沿用现有
"不支持图片格式 → 跳过 + ``ImageDropped`` 告警"降级;无预览图保持占位框。"""

from __future__ import annotations

import warnings
from pathlib import Path

import pdfspine
import pytest
from pptx_synth import REL_BASE, SlideSpec, build_pptx

import pptspine

PNG = (Path(__file__).parent / "fixtures" / "ocr_sample.png").read_bytes()
OLE_URI = "http://schemas.openxmlformats.org/presentationml/2006/ole"
MC = 'xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"'
PIC = (
    '<p:pic><p:nvPicPr><p:cNvPr id="3" name="Preview" descr="Sheet preview"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>'
    '<p:blipFill><a:blip r:embed="rId2"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>'
    '<p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1" cy="1"/></a:xfrm></p:spPr></p:pic>'
)
EMBED = '<p:oleObj name="Worksheet" r:id="rId3" progId="Excel.Sheet.12"><p:embed/></p:oleObj>'
WITH_PIC = f'<p:oleObj name="Worksheet" r:id="rId3" progId="Excel.Sheet.12"><p:embed/>{PIC}</p:oleObj>'
FRAME_RECT = (914_400, 914_400, 3_657_600, 2_743_200)


def _frame(inner: str) -> str:
    x, y, w, h = FRAME_RECT
    return (
        '<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Obj"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>'
        f'<p:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></p:xfrm>'
        f'<a:graphic><a:graphicData uri="{OLE_URI}">{inner}</a:graphicData></a:graphic></p:graphicFrame>'
    )


def _deck(sp_tree: str, media: bytes, ext: str) -> bytes:
    rels = [
        ("rId2", f"{REL_BASE}/image", f"../media/image1.{ext}"),
        ("rId3", f"{REL_BASE}/oleObject", "../embeddings/book1.xlsx"),
    ]
    return build_pptx([SlideSpec(sp_tree, rels)], {f"ppt/media/image1.{ext}": media})


def _export(pres: pptspine.Presentation) -> tuple[bytes, list[str]]:
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        pdf = pres.to_pdf()
    return pdf, [str(w.message) for w in caught]


def _wrapped(choice_frame: str, fallback_frame: str) -> str:
    return (
        f"<mc:AlternateContent {MC}><mc:Choice Requires=\"v\">{choice_frame}</mc:Choice>"
        f"<mc:Fallback>{fallback_frame}</mc:Fallback></mc:AlternateContent>"
    )


@pytest.mark.parametrize(
    "sp_tree",
    [_frame(WITH_PIC), _wrapped(_frame(EMBED), _frame(WITH_PIC))],
    ids=["direct", "alternate-content-frame"],
)
def test_ole_preview_is_a_picture_drawn_in_pdf(sp_tree: str) -> None:
    pres = pptspine.open_bytes(_deck(sp_tree, PNG, "png"))
    (shape,) = pres.slide(0).shapes()
    assert shape["kind"] == "picture"
    assert shape["rect"] == FRAME_RECT
    assert "![Sheet preview](image1.png)" in pres.to_markdown()
    pdf, caught = _export(pres)
    assert not any("mage" in m for m in caught), caught
    assert len(pdfspine.open(stream=pdf, filetype="pdf")[0].get_images()) == 1


def test_unsupported_emf_preview_degrades_with_image_dropped_warning() -> None:
    pres = pptspine.open_bytes(_deck(_frame(WITH_PIC), b"\x01\x00\x00\x00 not a real emf", "emf"))
    assert pres.slide(0).shapes()[0]["kind"] == "picture"
    pdf, caught = _export(pres)
    assert any("mage" in m for m in caught), caught
    assert pdf.startswith(b"%PDF-")
    assert len(pdfspine.open(stream=pdf, filetype="pdf")[0].get_images()) == 0


def test_ole_without_preview_stays_placeholder() -> None:
    pres = pptspine.open_bytes(_deck(_frame(EMBED), PNG, "png"))
    (shape,) = pres.slide(0).shapes()
    assert shape["kind"] == "placeholder" and shape["uri"] == OLE_URI
