"""SmartArt(``dgm:relIds``)文字进入模型与导出:有 drawing 部件 → frame 变换后的组合形状
(PDF 里与"拆组等价孪生"逐词同位),只有 data 部件 → 文字进 ``to_text`` / Markdown / dict,
PDF 保持占位框并发 ``smartart-degraded`` 告警。"""

from __future__ import annotations

import warnings

import pdfspine
import pytest
from pptx_synth import REL_BASE, SlideSpec, build_pptx

import pptspine

DGM = "http://schemas.openxmlformats.org/drawingml/2006/diagram"
REL_DRAWING = "http://schemas.microsoft.com/office/2007/relationships/diagramDrawing"
FRAME = (914_400, 1_828_800, 4_572_000, 2_286_000)  # off x, off y, ext cx, ext cy
CHILD = (457_200, 228_600, 2_743_200, 914_400)

FRAME_XML = f"""<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="D"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>
<p:xfrm><a:off x="{FRAME[0]}" y="{FRAME[1]}"/><a:ext cx="{FRAME[2]}" cy="{FRAME[3]}"/></p:xfrm>
<a:graphic><a:graphicData uri="{DGM}"><dgm:relIds xmlns:dgm="{DGM}" r:dm="rId2" r:lo="rId3" r:qs="rId4" r:cs="rId5"/></a:graphicData></a:graphic></p:graphicFrame>"""

RELS = [("rId2", f"{REL_BASE}/diagramData", "../diagrams/data1.xml"), ("rId6", REL_DRAWING, "../diagrams/drawing1.xml")]

A_NS = "http://schemas.openxmlformats.org/drawingml/2006/main"
DATA = f"""<dgm:dataModel xmlns:dgm="{DGM}" xmlns:a="{A_NS}"><dgm:ptLst>
<dgm:pt modelId="1" type="doc"><dgm:t><a:bodyPr/><a:p><a:endParaRPr/></a:p></dgm:t></dgm:pt>
<dgm:pt modelId="2"><dgm:t><a:bodyPr/><a:p><a:r><a:t>Alpha</a:t></a:r></a:p></dgm:t></dgm:pt>
<dgm:pt modelId="3" type="pres"><dgm:t><a:bodyPr/><a:p><a:r><a:t>HIDDEN</a:t></a:r></a:p></dgm:t></dgm:pt>
<dgm:pt modelId="4"><dgm:t><a:bodyPr/><a:p><a:r><a:t>Beta</a:t></a:r></a:p></dgm:t></dgm:pt>
</dgm:ptLst></dgm:dataModel>"""

TEXT_BODY = '<a:bodyPr/><a:p><a:r><a:rPr lang="en-US" sz="2000"><a:latin typeface="Arial"/></a:rPr><a:t>Alpha</a:t></a:r></a:p>'
DRAWING = f"""<dsp:drawing xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" xmlns:a="{A_NS}"><dsp:spTree>
<dsp:nvGrpSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvGrpSpPr/></dsp:nvGrpSpPr><dsp:grpSpPr/>
<dsp:sp><dsp:spPr><a:xfrm><a:off x="{CHILD[0]}" y="{CHILD[1]}"/><a:ext cx="{CHILD[2]}" cy="{CHILD[3]}"/></a:xfrm></dsp:spPr>
<dsp:txBody>{TEXT_BODY}</dsp:txBody></dsp:sp></dsp:spTree></dsp:drawing>"""

TWIN = f"""<p:sp><p:spPr><a:xfrm><a:off x="{FRAME[0] + CHILD[0]}" y="{FRAME[1] + CHILD[1]}"/>
<a:ext cx="{CHILD[2]}" cy="{CHILD[3]}"/></a:xfrm></p:spPr><p:txBody>{TEXT_BODY}</p:txBody></p:sp>"""


def _pdf(data: bytes) -> bytes:
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        return pptspine.open_bytes(data).to_pdf()


def _words(pdf: bytes) -> list:
    return pdfspine.open(stream=pdf, filetype="pdf")[0].get_text("words")


def test_drawing_part_renders_at_frame_transformed_position() -> None:
    deck = build_pptx(
        [SlideSpec(FRAME_XML, RELS)],
        {"ppt/diagrams/data1.xml": DATA, "ppt/diagrams/drawing1.xml": DRAWING},
    )
    twin = build_pptx([SlideSpec(TWIN)])
    wa, wb = _words(_pdf(deck)), _words(_pdf(twin))
    assert wa and [w[4] for w in wa] == [w[4] for w in wb] == ["Alpha"]
    for a, b in zip(wa[0][:4], wb[0][:4]):
        assert a == pytest.approx(b, abs=1.0)
    # 文字确实落在 frame 内(frame 左上 = (72, 144) pt)。
    assert wa[0][0] >= FRAME[0] / 12700 and wa[0][1] >= FRAME[1] / 12700
    pres = pptspine.open_bytes(deck)
    assert "Alpha" in pres.to_text() and "Alpha" in pres.to_markdown()
    assert "Beta" not in pres.to_text()  # drawing 优先,不叠加 data 文字
    (shape,) = pres.slide(0).shapes()
    assert shape["kind"] == "group"


def test_data_only_text_in_exports_dict_and_pdf_warns() -> None:
    deck = build_pptx([SlideSpec(FRAME_XML, RELS[:1])], {"ppt/diagrams/data1.xml": DATA})
    pres = pptspine.open_bytes(deck)
    assert "Alpha\nBeta" in pres.to_text()
    assert "HIDDEN" not in pres.to_text()
    md = pres.to_markdown()
    assert md.index("Alpha") < md.index("Beta") and "HIDDEN" not in md
    (shape,) = pres.slide(0).shapes()
    assert shape["kind"] == "placeholder" and shape["text"] == ["Alpha", "Beta"]
    with pytest.warns(UserWarning, match="smartart-degraded"):
        pres.to_pdf()


def test_both_parts_missing_and_malformed_do_not_crash() -> None:
    pres = pptspine.open_bytes(build_pptx([SlideSpec(FRAME_XML, RELS)]))
    (shape,) = pres.slide(0).shapes()
    assert shape["kind"] == "placeholder" and shape["text"] == []
    bad = build_pptx(
        [SlideSpec(FRAME_XML, RELS)],
        {"ppt/diagrams/data1.xml": "<<<not xml", "ppt/diagrams/drawing1.xml": "<dsp:drawing"},
    )
    assert pptspine.open_bytes(bad).slide(0).shapes()[0]["kind"] == "placeholder"


def test_expansion_budget_keyword_degrades_frames_beyond_budget() -> None:
    # 10 个 frame 指向同一 drawing(1 个形状):预算 3 => 3 个展开,7 个降级并记诊断,不抛错。
    data = build_pptx(
        [SlideSpec(FRAME_XML * 10, rels=RELS)],
        parts={"ppt/diagrams/data1.xml": DATA, "ppt/diagrams/drawing1.xml": DRAWING},
    )
    assert sum(s["kind"] == "group" for s in pptspine.open_bytes(data).slide(0).shapes()) == 10
    p = pptspine.open_bytes(data, max_diagram_shapes=3)
    kinds = [s["kind"] for s in p.slide(0).shapes()]
    assert kinds.count("group") == 3
    diag = [d for d in p.diagnostics() if d["kind"] == "smartart-degraded"]
    assert sum(d["count"] for d in diag) == 7

