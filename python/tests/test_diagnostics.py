"""解析诊断通道:``Presentation.diagnostics()`` 返回 ``[{"kind", "part", "count"}]``,
记录截断 / 缺失部件等"内容被静默丢失"的事实;完全正常的文件为空;只含种类 / 路径 / 计数,绝无正文。"""

from __future__ import annotations

from pptx_synth import NS, REL_BASE, SlideSpec, build_pptx

import pptspine

SECRET = "SECRET-BODY-TEXT"


def _sp(text: str) -> str:
    return (
        '<p:sp><p:nvSpPr><p:cNvPr id="2" name="T"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>'
        '<p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm></p:spPr>'
        f"<p:txBody><a:bodyPr/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"
    )


def test_clean_deck_has_no_diagnostics(minimal_pptx_bytes):
    assert pptspine.open_bytes(minimal_pptx_bytes).diagnostics() == []
    assert pptspine.open_bytes(build_pptx([SlideSpec(_sp("hi"))])).diagnostics() == []


def test_truncated_slide_is_reported_and_prefix_survives():
    full = f"<p:sld {NS}><p:cSld><p:spTree>{_sp('KEPT')}{_sp(SECRET)}</p:spTree></p:cSld></p:sld>"
    cut = full[: full.index(SECRET) - 40]
    pres = pptspine.open_bytes(build_pptx([SlideSpec(_sp("x"))], {"ppt/slides/slide1.xml": cut}))
    assert "KEPT" in pres.to_text()
    diags = pres.diagnostics()
    assert [d["kind"] for d in diags] == ["xml-truncated"]
    assert diags[0]["part"] == "ppt/slides/slide1.xml"
    assert isinstance(diags[0]["count"], int) and diags[0]["count"] > 0
    assert SECRET not in repr(diags)


def test_dangling_relationship_is_reported_per_source_part():
    spec = SlideSpec(_sp("x"), [("rId9", f"{REL_BASE}/image", "../media/missing.png")])
    diags = pptspine.open_bytes(build_pptx([spec])).diagnostics()
    assert diags == [{"kind": "missing-part", "part": "ppt/slides/slide1.xml", "count": 1}]
