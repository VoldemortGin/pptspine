"""pptspine 结构化解析的验收测试 —— 对合成的最小 ``.pptx`` 断言文字 / 表格 / 几何。"""

from __future__ import annotations

import pytest

import pptspine


def test_open_path_basic(minimal_pptx_path, slide_canvas):
    pres = pptspine.open(minimal_pptx_path)
    assert pres.slide_count == 1
    assert len(pres) == 1
    assert pres.slide_size == slide_canvas


def test_open_bytes_matches_path(minimal_pptx_bytes, slide_canvas):
    pres = pptspine.open_bytes(minimal_pptx_bytes)
    assert pres.slide_count == 1
    assert pres.slide_size == slide_canvas
    # 画布磅尺寸:9144000 EMU / 12700 = 720 pt;6858000 / 12700 = 540 pt。
    w_pt, h_pt = pres.slide_size_points
    assert w_pt == pytest.approx(720.0)
    assert h_pt == pytest.approx(540.0)


def test_slide_handle(minimal_pptx_bytes):
    pres = pptspine.open_bytes(minimal_pptx_bytes)
    slides = pres.slides()
    assert len(slides) == 1
    assert slides[0].index == 0
    # 越界抛 IndexError。
    with pytest.raises(IndexError):
        pres.slide(5)


def test_textbox_runs_and_styling(minimal_pptx_bytes):
    pres = pptspine.open_bytes(minimal_pptx_bytes)
    shapes = pres.slides()[0].shapes()
    text_shapes = [s for s in shapes if s["kind"] == "text"]
    assert len(text_shapes) == 1
    tf = text_shapes[0]

    # 两段:标题 + 副行。
    assert tf["text"] == "Hello pptspine\nsecond line"
    paras = tf["paragraphs"]
    assert len(paras) == 2

    # 第一段:居中、带样式的标题 run。
    title_para = paras[0]
    assert title_para["align"] == "ctr"
    title_run = title_para["runs"][0]
    assert title_run["text"] == "Hello pptspine"
    assert title_run["bold"] is True
    assert title_run["italic"] is False
    assert title_run["size_pt"] == pytest.approx(44.0)  # sz="4400" / 100
    assert title_run["font"] == "Calibri"
    assert title_run["color"] == "1F4E79"

    # 第二段:无样式。
    assert paras[1]["runs"][0]["text"] == "second line"
    assert paras[1]["runs"][0]["size_pt"] == pytest.approx(20.0)


def test_textbox_geometry(minimal_pptx_bytes):
    pres = pptspine.open_bytes(minimal_pptx_bytes)
    tf = [s for s in pres.slides()[0].shapes() if s["kind"] == "text"][0]
    # EMU 矩形原样保留。
    assert tf["rect"] == (838200, 365125, 7772400, 1325563)
    # 磅换算便利字段。
    x_pt, y_pt, w_pt, h_pt = tf["rect_points"]
    assert x_pt == pytest.approx(838200 / 12700)
    assert w_pt == pytest.approx(7772400 / 12700)


def test_table_rows_cells(minimal_pptx_bytes):
    pres = pptspine.open_bytes(minimal_pptx_bytes)
    shapes = pres.slides()[0].shapes()
    tables = [s for s in shapes if s["kind"] == "table"]
    assert len(tables) == 1
    table = tables[0]

    rows = table["rows"]
    assert len(rows) == 2
    # 便利的逐行文字。
    assert rows[0]["text"] == ["A1", "B1"]
    assert rows[1]["text"] == ["A2", "B2"]

    # 单元格元数据:默认跨度、首格填充色。
    a1 = rows[0]["cells"][0]
    assert a1["text"] == "A1"
    assert a1["col_span"] == 1
    assert a1["row_span"] == 1
    assert a1["merged"] is False
    assert a1["fill"] == "FFCC00"
    # 无填充的格 fill 为 None。
    assert rows[0]["cells"][1]["fill"] is None


def test_table_geometry(minimal_pptx_bytes):
    pres = pptspine.open_bytes(minimal_pptx_bytes)
    table = [s for s in pres.slides()[0].shapes() if s["kind"] == "table"][0]
    assert table["rect"] == (838200, 2000250, 7772400, 2000250)


def test_malformed_input_raises_typed_error():
    # 不是 zip -> 收敛成类型化 PptZipError(PptError 子类),绝不 panic。
    with pytest.raises(pptspine.PptError):
        pptspine.open_bytes(b"this is definitely not a pptx zip")
    with pytest.raises(pptspine.PptZipError):
        pptspine.open_bytes(b"\x00\x01\x02\x03 not a zip")


def test_open_missing_file_raises():
    with pytest.raises((FileNotFoundError, OSError)):
        pptspine.open("/no/such/deck-12345.pptx")
