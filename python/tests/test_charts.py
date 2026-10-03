"""图表数据抽取验收:``ppt/charts/chartN.xml`` 的缓存(类别 / 系列 / 值)进入 ``shapes()`` dict、
``to_markdown()`` 表格与 ``to_text()`` 行(对标 docling chart→TableData、MarkItDown 图表转表格)。"""

from __future__ import annotations

import warnings

import pptspine


def _chart(pres: pptspine.Presentation, slide: int) -> dict:
    (ph,) = [s for s in pres.slide(slide).shapes() if s["kind"] == "placeholder"]
    assert ph["uri"].endswith("/chart")
    assert ph["chart"] is not None
    return ph["chart"]


def _md_slide(md: str, n: int) -> str:
    start = md.index(f"## Slide {n}")
    end = md.find("## Slide ", start + 1)
    return md[start : end if end != -1 else None]


def test_bar_chart_dict(chart_pptx_bytes):
    c = _chart(pptspine.open_bytes(chart_pptx_bytes), 0)
    assert c["kind"] == "bar"
    assert c["title"] == "Quarterly Sales"
    assert c["categories"] == ["Q1", "Q2", "Q3"]
    assert c["series"] == [
        {"name": "North", "values": [10.0, 20.5, 30.0], "format_code": "General"},
        {"name": "South", "values": [4.0, 5.0, 6.0], "format_code": "General"},
    ]
    assert c["warnings"] == []


def test_bar_chart_markdown_table_follows_reading_order(chart_pptx_bytes):
    md = _md_slide(pptspine.open_bytes(chart_pptx_bytes).to_markdown(), 1)
    table = (
        "#### Chart: Quarterly Sales\n\n"
        "| Category | North | South |\n"
        "| --- | --- | --- |\n"
        "| Q1 | 10 | 4 |\n"
        "| Q2 | 20.5 | 5 |\n"
        "| Q3 | 30 | 6 |"
    )
    assert table in md, md
    assert md.index("Intro text") < md.index("#### Chart")


def test_bar_chart_text_lines(chart_pptx_bytes):
    text = pptspine.open_bytes(chart_pptx_bytes).slide(0).text
    assert text.splitlines() == [
        "Intro text",
        "Quarterly Sales",
        "Q1: 10, 4",
        "Q2: 20.5, 5",
        "Q3: 30, 6",
    ]


def test_pie_chart_single_series_with_percent_format(chart_pptx_bytes):
    pres = pptspine.open_bytes(chart_pptx_bytes)
    c = _chart(pres, 1)
    assert c["kind"] == "pie"
    assert c["title"] == "Fruit Mix"
    assert c["categories"] == ["Apples", "Pears", "Plums"]
    assert c["series"] == [{"name": "Share", "values": [0.25, 0.5, 0.25], "format_code": "0%"}]
    md = _md_slide(pres.to_markdown(), 2)
    assert (
        "#### Chart: Fruit Mix\n\n| Category | Share |\n| --- | --- |\n"
        "| Apples | 25% |\n| Pears | 50% |\n| Plums | 25% |"
    ) in md, md


def test_scatter_chart_uses_x_and_y_values_without_title(chart_pptx_bytes):
    pres = pptspine.open_bytes(chart_pptx_bytes)
    c = _chart(pres, 2)
    assert c["kind"] == "scatter"
    assert c["title"] is None
    assert c["categories"] == ["1", "2", "4"]
    assert c["series"][0]["values"] == [1.5, 3.0, 6.25]
    md = _md_slide(pres.to_markdown(), 3)
    assert (
        "#### Chart (scatter)\n\n| X | Growth |\n| --- | --- |\n"
        "| 1 | 1.5 |\n| 2 | 3 |\n| 4 | 6.25 |"
    ) in md, md
    assert pres.slide(2).text.splitlines()[0] == "Chart (scatter)"


def test_sparse_points_and_missing_cache(chart_pptx_bytes):
    pres = pptspine.open_bytes(chart_pptx_bytes)
    c = _chart(pres, 3)
    assert c["kind"] == "line"
    assert c["title"] is None
    assert c["categories"] == ["Jan", "Feb", "Mar", "Apr"]
    dense, linked = c["series"]
    assert dense["values"] == [1.0, None, 3.0, None]
    assert linked == {"name": "Linked", "values": [], "format_code": None}
    assert any("Linked" in w and "cache missing" in w for w in c["warnings"]), c["warnings"]
    md = _md_slide(pres.to_markdown(), 4)
    assert (
        "#### Chart (line)\n\n| Category | Dense | Linked |\n| --- | --- | --- |\n"
        "| Jan | 1 |  |\n| Feb |  |  |\n| Mar | 3 |  |\n| Apr |  |  |"
    ) in md, md


def test_non_chart_placeholder_has_none_chart(b3_pptx_bytes):
    """无 rels / 部件缺失的图表帧:``chart`` 为 ``None``,占位行为不变。"""
    pres = pptspine.open_bytes(b3_pptx_bytes)
    (ph,) = [s for s in pres.slide(0).shapes() if s["kind"] == "placeholder"]
    assert ph["chart"] is None
    assert "Chart" not in pres.to_markdown()


def test_chart_frames_still_render_as_placeholder(chart_pptx_bytes):
    """渲染不画图表(仍是占位框):带图表数据的 deck 照常导出 PDF。"""
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")  # 字体替换告警与本测试无关
        pdf = pptspine.open_bytes(chart_pptx_bytes).to_pdf()
    assert pdf.startswith(b"%PDF")
