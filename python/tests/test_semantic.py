"""语义抽取验收:视觉阅读顺序、标题占位符、列表标记、图片 alt、超链接、隐藏页、节、
文档属性、``shapes()`` 的 ``placeholder`` 键(对标 MarkItDown / docling / unstructured)。"""

from __future__ import annotations

import warnings

import pdfspine
import pytest

import pptspine


def _order(text: str, *needles: str) -> list[int]:
    idx = [text.index(n) for n in needles]
    return idx


# --- 1. 视觉阅读顺序 ------------------------------------------------------------


def test_visual_order_reads_left_column_before_right(two_column_pptx_bytes):
    out = pptspine.open_bytes(two_column_pptx_bytes).to_text()
    pos = _order(out, "Two Columns", "L1", "L2", "L3", "R1", "R2")
    assert pos == sorted(pos), out


def test_visual_order_is_default_and_document_order_is_available(two_column_pptx_bytes):
    pres = pptspine.open_bytes(two_column_pptx_bytes)
    assert pres.to_text() == pres.to_text(order="visual")
    doc = pres.to_text(order="document")
    pos = _order(doc, "R1", "L1", "R2", "L2", "L3", "Two Columns")
    assert pos == sorted(pos), doc


def test_slide_text_uses_visual_order(two_column_pptx_bytes):
    text = pptspine.open_bytes(two_column_pptx_bytes).slide(0).text
    assert text.splitlines() == ["Two Columns", "L1", "L2", "L3", "R1", "R2"]


def test_markdown_follows_order_parameter(two_column_pptx_bytes):
    pres = pptspine.open_bytes(two_column_pptx_bytes)
    md = pres.to_markdown()
    assert md.startswith("## Slide 1\n\n### Two Columns\n\nL1\n\nL2\n\nL3\n\nR1\n\nR2"), md
    # 文档顺序下标题占位符仍作 `###`,正文按 spTree 顺序。
    md_doc = pres.to_markdown(order="document")
    assert md_doc.startswith("## Slide 1\n\n### Two Columns\n\nR1\n\nL1\n\nR2"), md_doc


def test_invalid_order_raises_value_error(two_column_pptx_bytes):
    pres = pptspine.open_bytes(two_column_pptx_bytes)
    with pytest.raises(ValueError):
        pres.to_text(order="zorder")
    with pytest.raises(ValueError):
        pres.to_markdown(order="")


# --- 2/3/4/5. Markdown:标题 / 列表 / 图片 / 超链接 -------------------------------


@pytest.fixture(scope="module")
def semantic_md(semantic_pptx_bytes) -> str:
    return pptspine.open_bytes(semantic_pptx_bytes).to_markdown()


def test_title_comes_from_title_placeholder(semantic_md):
    # 最上方的非占位符横幅不再被当成标题;标题段内换行折成空格。
    assert semantic_md.startswith("## Slide 1\n\n### Agenda Q3\n\nCONFIDENTIAL"), semantic_md
    assert "### CONFIDENTIAL" not in semantic_md


def test_ctr_title_and_subtitle_as_plain_paragraph(semantic_pptx_bytes):
    md = pptspine.open_bytes(semantic_pptx_bytes).to_markdown()
    assert "## Slide 3\n\n### Appendix\n\nBackup material" in md
    assert "- Backup material" not in md


def test_list_markers_follow_resolved_bullets(semantic_md):
    # master bodyStyle:lvl1 buChar •、lvl2 buChar –;段落直接 buNone / buAutoNum 覆盖。
    assert "- Revenue up\n  - Cloud detail" in semantic_md
    assert "\n\nNo marker here\n\n" in semantic_md
    assert "1. First step\n2. Second step" in semantic_md


def test_pictures_render_with_alt_text_or_name(semantic_md):
    assert "[![Revenue chart](image1.png)](https://example.com/chart)" in semantic_md
    assert "![Logo 5](image1.png)" in semantic_md


def test_external_links_markdown_internal_jump_plain(semantic_md):
    assert "Visit [our site](https://example.com/home) or the appendix" in semantic_md
    assert "[the appendix]" not in semantic_md


def test_to_text_keeps_plain_text(semantic_pptx_bytes):
    out = pptspine.open_bytes(semantic_pptx_bytes).to_text()
    assert "Visit our site or the appendix" in out
    assert "](" not in out


# --- 4/5/7. shapes() dict:placeholder / alt_text / title / hyperlink --------------


def test_shapes_expose_placeholder(semantic_pptx_bytes):
    pres = pptspine.open_bytes(semantic_pptx_bytes)
    shapes = pres.slide(0).shapes()
    assert shapes[0]["placeholder"] is None
    assert shapes[1]["placeholder"] == {"type": "title", "idx": None}
    # `p:ph` 缺省 type 按 ECMA-376 记 body。
    assert shapes[2]["placeholder"] == {"type": "body", "idx": 1}
    assert pres.slide(2).shapes()[1]["placeholder"] == {"type": "subTitle", "idx": 1}


def test_picture_alt_text_and_title_keys(semantic_pptx_bytes):
    shapes = pptspine.open_bytes(semantic_pptx_bytes).slide(0).shapes()
    pic, logo = shapes[3], shapes[4]
    assert pic["kind"] == "picture"
    assert (pic["alt_text"], pic["title"], pic["name"]) == ("Revenue chart", "Chart", "Picture 4")
    assert pic["hyperlink"]["url"] == "https://example.com/chart"
    assert (logo["alt_text"], logo["title"], logo["hyperlink"]) == (None, None, None)


def test_run_hyperlink_dicts(semantic_pptx_bytes):
    body = pptspine.open_bytes(semantic_pptx_bytes).slide(0).shapes()[2]
    runs = body["paragraphs"][5]["runs"]
    assert runs[0]["hyperlink"] is None
    assert runs[1]["hyperlink"] == {
        "url": "https://example.com/home",
        "slide_index": None,
        "action": None,
        "tooltip": "home",
    }
    assert runs[3]["hyperlink"] == {
        "url": None,
        "slide_index": 2,
        "action": "ppaction://hlinksldjump",
        "tooltip": None,
    }
    assert body["hyperlink"] is None


# --- 6. 隐藏页 / 节 / 文档属性 --------------------------------------------------


def test_hidden_flag_and_default_skip(semantic_pptx_bytes):
    pres = pptspine.open_bytes(semantic_pptx_bytes)
    assert [s.hidden for s in pres.slides()] == [False, True, False]
    out = pres.to_text()
    assert "SECRET DRAFT" not in out and "--- slide 2 ---" not in out
    assert "--- slide 3 ---" in out  # 序号不重排
    assert "## Slide 2" not in pres.to_markdown()
    assert "SECRET DRAFT" in pres.to_text(include_hidden=True)
    assert "## Slide 2" in pres.to_markdown(include_hidden=True)


def test_pdf_skips_hidden_slides_by_default(semantic_pptx_bytes, tmp_path):
    pres = pptspine.open_bytes(semantic_pptx_bytes)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        default = pres.to_pdf()
        full = pres.to_pdf(include_hidden=True)
        out = tmp_path / "full.pdf"
        pres.save_pdf(out, include_hidden=True)
    assert pdfspine.open(stream=default, filetype="pdf").page_count == 2
    assert pdfspine.open(stream=full, filetype="pdf").page_count == 3
    assert out.read_bytes() == full


def test_sections(semantic_pptx_bytes, minimal_pptx_bytes):
    assert pptspine.open_bytes(semantic_pptx_bytes).sections() == [
        ("Intro", [0]),
        ("Rest", [1, 2]),
    ]
    assert pptspine.open_bytes(minimal_pptx_bytes).sections() == []


def test_core_properties(semantic_pptx_bytes, minimal_pptx_bytes):
    props = pptspine.open_bytes(semantic_pptx_bytes).core_properties()
    assert props["title"] == "Quarterly Business Review"
    assert props["creator"] == "Ada Lovelace"
    assert props["last_modified_by"] == "Charles Babbage"
    assert props["revision"] == "3"
    assert props["created"] == "2026-09-01T08:00:00Z"
    assert props["modified"] == "2026-09-30T17:30:00Z"
    assert props["application"] == "Microsoft Office PowerPoint"
    assert props["company"] == "Analytical Engines"
    assert props["subject"] is None
    empty = pptspine.open_bytes(minimal_pptx_bytes).core_properties()
    assert set(empty) == set(props)
    assert all(v is None for v in empty.values())
