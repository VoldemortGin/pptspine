"""批注:旧式 ``p:cmLst`` 与新式线程批注(含回复)经 ``Slide.comments()`` 暴露;批注是审阅
元数据,默认不进 ``text`` / ``to_text()`` / ``to_markdown()`` / PDF,也不进任何告警。"""

from __future__ import annotations

import warnings

import pdfspine
from pptx_synth import REL_BASE, SlideSpec, build_pptx

import pptspine

P188 = "http://schemas.microsoft.com/office/powerpoint/2018/8/main"
REL_MS = "http://schemas.microsoft.com/office/2018/10/relationships"
BODY = (
    '<p:sp><p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="4572000" cy="914400"/></a:xfrm></p:spPr>'
    '<p:txBody><a:bodyPr/><a:p><a:r><a:t>Visible body</a:t></a:r></a:p></p:txBody></p:sp>'
)
LEGACY = (
    '<p:cmLst xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">'
    '<p:cm authorId="0" dt="2024-05-06T10:11:12.000" idx="1"><p:pos x="10" y="20"/>'
    "<p:text>SECRET legacy note</p:text></p:cm></p:cmLst>"
)
LEGACY_AUTHORS = (
    '<p:cmAuthorLst xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">'
    '<p:cmAuthor id="0" name="Zed Zimmer" initials="ZZ" lastIdx="1" clrIdx="0"/></p:cmAuthorLst>'
)
MODERN = (
    f'<p188:cmLst xmlns:p188="{P188}" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">'
    '<p188:cm id="{C}" authorId="{G}" created="2024-06-01T09:00:00.000"><p188:replyLst>'
    '<p188:reply id="{R}" authorId="{G}"><p188:txBody><a:bodyPr/><a:p><a:r><a:t>SECRET reply</a:t></a:r></a:p></p188:txBody></p188:reply>'
    "</p188:replyLst><p188:txBody><a:bodyPr/><a:p><a:r><a:t>SECRET modern</a:t></a:r></a:p></p188:txBody></p188:cm></p188:cmLst>"
)
MODERN_AUTHORS = (
    f'<p188:authorLst xmlns:p188="{P188}"><p188:author id="{{G}}" name="Yan Young" initials="YY"/></p188:authorLst>'
)


def _legacy() -> bytes:
    return build_pptx(
        [SlideSpec(BODY, [("rId5", f"{REL_BASE}/comments", "../comments/comment1.xml")])],
        {"ppt/comments/comment1.xml": LEGACY, "ppt/commentAuthors.xml": LEGACY_AUTHORS},
        pres_rels=[("rId90", f"{REL_BASE}/commentAuthors", "commentAuthors.xml")],
    )


def test_legacy_comment_dict() -> None:
    pres = pptspine.open_bytes(_legacy())
    (c,) = pres.slide(0).comments()
    assert c == {
        "author": "Zed Zimmer",
        "initials": "ZZ",
        "datetime": "2024-05-06T10:11:12.000",
        "text": "SECRET legacy note",
        "position": (10, 20),
        "replies": [],
    }


def test_modern_threaded_comment_with_reply() -> None:
    deck = build_pptx(
        [SlideSpec(BODY, [("rId5", f"{REL_MS}/comments", "../comments/modernComment_1.xml")])],
        {"ppt/comments/modernComment_1.xml": MODERN, "ppt/authors.xml": MODERN_AUTHORS},
        pres_rels=[("rId90", f"{REL_MS}/authors", "authors.xml")],
    )
    (c,) = pptspine.open_bytes(deck).slide(0).comments()
    assert (c["author"], c["initials"], c["text"], c["position"]) == ("Yan Young", "YY", "SECRET modern", None)
    assert c["datetime"] == "2024-06-01T09:00:00.000"
    (r,) = c["replies"]
    assert (r["author"], r["text"], r["datetime"]) == ("Yan Young", "SECRET reply", None)


def test_no_comments_is_empty_list() -> None:
    assert pptspine.open_bytes(build_pptx([SlideSpec(BODY)])).slide(0).comments() == []


def test_default_exports_and_pdf_exclude_comments_and_warnings_stay_clean() -> None:
    pres = pptspine.open_bytes(_legacy())
    assert pres.slide(0).comments()
    for out in (pres.to_text(), pres.to_markdown(), pres.slide(0).text):
        assert "Visible body" in out
        assert "SECRET" not in out and "Zed" not in out
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        pdf = pres.to_pdf()
    assert not any("SECRET" in str(w.message) or "Zed" in str(w.message) for w in caught)
    page_text = pdfspine.open(stream=pdf, filetype="pdf")[0].get_text()
    assert "Visible body" in page_text and "SECRET" not in page_text
