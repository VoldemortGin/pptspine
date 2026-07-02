"""结构化导出验收:``to_text`` / ``to_markdown`` / ``Slide.text``,覆盖合并单元格表格。"""

from __future__ import annotations

import pptspine


def test_slide_text_property(minimal_pptx_bytes):
    pres = pptspine.open_bytes(minimal_pptx_bytes)
    slide = pres.slides()[0]
    text = slide.text
    # 文本框两段 + 表格文字都应在内。
    assert "Hello pptspine" in text
    assert "second line" in text
    assert "A1 | B1" in text
    assert "A2 | B2" in text


def test_to_text_slide_separators(minimal_pptx_bytes):
    pres = pptspine.open_bytes(minimal_pptx_bytes)
    out = pres.to_text()
    assert "--- slide 1 ---" in out
    assert "Hello pptspine" in out


def test_to_markdown_basic_and_gfm_table(minimal_pptx_bytes):
    pres = pptspine.open_bytes(minimal_pptx_bytes)
    md = pres.to_markdown()
    # 每页一节。
    assert "## Slide 1" in md
    # 首个文本框第一段作标题。
    assert "### Hello pptspine" in md
    # 无合并的表格走 GFM。
    assert "| A1 | B1 |" in md
    assert "| --- | --- |" in md
    assert "| A2 | B2 |" in md
    # 不应出现 HTML 表格。
    assert "<table>" not in md


def test_to_markdown_merged_table_uses_html(merged_table_pptx_bytes):
    pres = pptspine.open_bytes(merged_table_pptx_bytes)
    md = pres.to_markdown()
    # 含合并单元格 -> HTML <table> 保真。
    assert "<table>" in md
    assert '<td colspan="2">Header</td>' in md
    assert "<td>A2</td>" in md
    assert "<td>B2</td>" in md
    # 被合并掉的延续格不应单独输出(三个 <td>)。
    assert md.count("<td") == 3


def test_b3_to_text_recovers_lost_text(b3_pptx_bytes):
    """B-3 止损后 ``to_text`` 找回此前默默丢掉的文字:``a:br`` 后的第二行、
    ``a:fld`` 缓存文本、``mc:Fallback`` 里的形状文字;Choice 分支不混入。"""
    pres = pptspine.open_bytes(b3_pptx_bytes)
    out = pres.to_text()
    assert "Line one\nLine two" in out
    assert "Fallback shape" in out
    assert "NEWER CHOICE" not in out
    assert "A1 | B1" in out
