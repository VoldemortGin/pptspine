"""演讲者备注验收:解析 notesSlide、暴露 ``Slide.notes``、并纳入 ``to_text`` / ``to_markdown``。"""

from __future__ import annotations

import pptspine


def test_slide_notes_extracted(notes_pptx):
    pptx_bytes, expected = notes_pptx
    pres = pptspine.open_bytes(pptx_bytes)
    slide = pres.slides()[0]
    assert slide.notes == expected
    # 非 body 占位符的文字不应混入备注。
    assert "NOT THE NOTES" not in slide.notes


def test_slides_without_notes_are_none(minimal_pptx_bytes):
    pres = pptspine.open_bytes(minimal_pptx_bytes)
    assert pres.slides()[0].notes is None


def test_notes_in_text_and_markdown(notes_pptx):
    pptx_bytes, _expected = notes_pptx
    pres = pptspine.open_bytes(pptx_bytes)

    text = pres.to_text()
    assert "Notes:" in text
    assert "Remember to smile" in text

    md = pres.to_markdown()
    assert "> Notes:" in md
    assert "> Remember to smile" in md
    assert "> Second note line" in md
