"""页脚 / 页码 / 日期占位符的 PDF 渲染:``slidenum`` 求值为显示页码(放映序号 + ``firstSlideNum`` − 1,
隐藏页仍占号),``datetime*`` 用字段缓存文本且渲染逐字节确定,版式 / 母版上的占位符只有幻灯片
实例化了才出现,页脚文字取幻灯片自身、位置沿版式占位符继承。"""

from __future__ import annotations

import warnings

import pdfspine
from pptx_synth import NS, REL_BASE, SlideSpec, build_pptx, rels_xml

import pptspine

EMU = 12700.0
SLDNUM_X, SLDNUM_Y = 7_000_000, 6_200_000
FTR_X, FTR_Y = 2_000_000, 6_200_000
DT_X, DT_Y = 300_000, 6_200_000


def _ph(ty: str, idx: int, xy: tuple[int, int] | None, body: str) -> str:
    xfrm = (
        f'<a:xfrm><a:off x="{xy[0]}" y="{xy[1]}"/><a:ext cx="1800000" cy="400000"/></a:xfrm>' if xy else ""
    )
    return (
        f'<p:sp><p:nvSpPr><p:cNvPr id="{idx}" name="{ty}"/><p:cNvSpPr/><p:nvPr><p:ph type="{ty}" idx="{idx}"/></p:nvPr></p:nvSpPr>'
        f"<p:spPr>{xfrm}</p:spPr><p:txBody><a:bodyPr/><a:lstStyle/>{body}</p:txBody></p:sp>"
    )


def _fld(ty: str, cached: str) -> str:
    return f'<a:p><a:fld id="{{F}}" type="{ty}"><a:rPr lang="en-US" sz="1400"/><a:t>{cached}</a:t></a:fld></a:p>'


def _text(t: str) -> str:
    return f'<a:p><a:r><a:rPr lang="en-US" sz="1400"/><a:t>{t}</a:t></a:r></a:p>'


def _template() -> dict[str, str]:
    tpl = (
        _ph("dt", 10, (DT_X, DT_Y), _fld("datetimeFigureOut", "TEMPLATE-DATE"))
        + _ph("ftr", 11, (FTR_X, FTR_Y), _text("TEMPLATE-FOOTER"))
        + _ph("sldNum", 12, (SLDNUM_X, SLDNUM_Y), _fld("slidenum", "TEMPLATE-NUM"))
    )
    return {
        "ppt/slideMasters/slideMaster1.xml": (
            f"<p:sldMaster {NS}><p:cSld><p:spTree>{tpl}</p:spTree></p:cSld>"
            '<p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" '
            'accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/></p:sldMaster>'
        ),
        "ppt/slideMasters/_rels/slideMaster1.xml.rels": rels_xml([]),
        "ppt/slideLayouts/slideLayout1.xml": (
            f"<p:sldLayout {NS}><p:cSld><p:spTree>{tpl}</p:spTree></p:cSld></p:sldLayout>"
        ),
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels": rels_xml(
            [("rId1", f"{REL_BASE}/slideMaster", "../slideMasters/slideMaster1.xml")]
        ),
    }


LAYOUT_REL = [("rId1", f"{REL_BASE}/slideLayout", "../slideLayouts/slideLayout1.xml")]


def _slide(sp_tree: str, hidden: bool = False) -> SlideSpec:
    return SlideSpec(sp_tree, LAYOUT_REL, ' show="0"' if hidden else "")


def _num() -> str:
    return _ph("sldNum", 12, None, _fld("slidenum", "‹#›"))


def _pdf(slides: list[SlideSpec], pres_attrs: str = "") -> bytes:
    deck = build_pptx(slides, _template(), pres_attrs=" " + pres_attrs if pres_attrs else "")
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        return pptspine.open_bytes(deck).to_pdf()


def _words(pdf: bytes, page: int) -> list:
    return pdfspine.open(stream=pdf, filetype="pdf")[page].get_text("words")


def test_page_numbers_follow_slide_order_at_layout_position() -> None:
    pdf = _pdf([_slide(_num()), _slide(_num()), _slide(_num())])
    doc = pdfspine.open(stream=pdf, filetype="pdf")
    assert len(doc) == 3
    for i in range(3):
        words = _words(pdf, i)
        assert [w[4] for w in words] == [str(i + 1)], words
        # 位置沿版式 sldNum 占位符(左内边距 7.2pt)。
        assert words[0][0] >= SLDNUM_X / EMU and words[0][0] < SLDNUM_X / EMU + 20
        assert words[0][1] >= SLDNUM_Y / EMU


def test_first_slide_num_is_honoured() -> None:
    pdf = _pdf([_slide(_num()), _slide(_num())], pres_attrs='firstSlideNum="10"')
    assert [w[4] for w in _words(pdf, 0)] == ["10"]
    assert [w[4] for w in _words(pdf, 1)] == ["11"]


def test_hidden_slide_keeps_its_number_and_is_skipped() -> None:
    pdf = _pdf([_slide(_num()), _slide(_num(), hidden=True), _slide(_num())])
    assert len(pdfspine.open(stream=pdf, filetype="pdf")) == 2
    assert [w[4] for w in _words(pdf, 0)] == ["1"]
    assert [w[4] for w in _words(pdf, 1)] == ["3"]


def test_datetime_uses_cached_text_and_render_is_byte_deterministic() -> None:
    dt = _ph("dt", 10, None, _fld("datetime1", "1/2/2020"))
    slides = [_slide(dt)]
    first, second = _pdf(slides), _pdf(slides)
    assert first == second
    assert [w[4] for w in _words(first, 0)] == ["1/2/2020"]


def test_template_placeholders_are_not_drawn_when_slide_has_none() -> None:
    pdf = _pdf([_slide(_ph("body", 1, (500_000, 500_000), _text("Only content")))])
    texts = [w[4] for w in _words(pdf, 0)]
    assert texts == ["Only", "content"], texts
    assert not any("TEMPLATE" in t for t in texts)


def test_footer_text_from_slide_at_layout_position() -> None:
    pdf = _pdf([_slide(_ph("ftr", 11, None, _text("Quarterly review")))])
    words = _words(pdf, 0)
    assert [w[4] for w in words] == ["Quarterly", "review"]
    assert FTR_X / EMU <= words[0][0] < FTR_X / EMU + 20
    assert not any("TEMPLATE" in w[4] for w in words)
