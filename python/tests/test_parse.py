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


# --- B-3 解析止损批(PRD-PDF-EXPORT §3.h/i/l/p/s/t/u)-------------------------------


def _b3_shapes(b3_pptx_bytes):
    return pptspine.open_bytes(b3_pptx_bytes).slides()[0].shapes()


def test_b3_br_and_fld_runs(b3_pptx_bytes):
    """段内换行 ``a:br`` 与字段 ``a:fld`` 不再丢文本,run 上带 kind 标记。"""
    tf = [s for s in _b3_shapes(b3_pptx_bytes) if s["kind"] == "text"][0]

    para0 = tf["paragraphs"][0]
    kinds = [r["kind"] for r in para0["runs"]]
    assert kinds == ["text", "break", "text"]
    assert para0["text"] == "Line one\nLine two"

    fld = tf["paragraphs"][1]["runs"][0]
    assert fld["kind"] == "field"
    assert fld["field_type"] == "slidenum"
    assert fld["text"] == "1"


def test_b3_run_fonts_and_decorations(b3_pptx_bytes):
    """``a:ea``/``a:cs`` 字体与 ``@u``/``@strike`` 落进 run dict。"""
    tf = [s for s in _b3_shapes(b3_pptx_bytes) if s["kind"] == "text"][0]
    styled = tf["paragraphs"][0]["runs"][2]
    assert styled["font"] == "Calibri"
    assert styled["ea_font"] == "SimSun"
    assert styled["cs_font"] == "Arial"
    assert styled["underline"] is True
    assert styled["strike"] is True
    # 普通 run 缺省全为关闭 / None。
    plain = tf["paragraphs"][0]["runs"][0]
    assert plain["underline"] is False
    assert plain["strike"] is False
    assert plain["ea_font"] is None


def test_b3_alternate_content_takes_fallback(b3_pptx_bytes):
    """``mc:AlternateContent`` 降入 Fallback:形状不再整块消失,Choice 被跳过。"""
    texts = [s["text"] for s in _b3_shapes(b3_pptx_bytes) if s["kind"] == "text"]
    assert any("Fallback shape" in t for t in texts)
    assert not any("NEWER CHOICE" in t for t in texts)


def test_b3_connector_shape(b3_pptx_bytes):
    """连接线 ``p:cxnSp`` 以 kind="connector" 呈现,带几何 / 矩形 / 描边三件套。"""
    conns = [s for s in _b3_shapes(b3_pptx_bytes) if s["kind"] == "connector"]
    assert len(conns) == 1
    c = conns[0]
    assert c["geometry"] == "straightConnector1"
    assert c["rect"] == (100, 200, 300, 400)
    assert c["stroke"] == "FF0000"
    assert c["stroke_width_emu"] == 19050
    assert c["stroke_dash"] == "dash"


def test_b3_chart_placeholder_keeps_rect(b3_pptx_bytes):
    """非表格 graphicFrame(图表)降级为 kind="placeholder",外框矩形保住。"""
    phs = [s for s in _b3_shapes(b3_pptx_bytes) if s["kind"] == "placeholder"]
    assert len(phs) == 1
    ph = phs[0]
    assert ph["rect"] == (1000, 2000, 3000, 4000)
    assert ph["uri"].endswith("/chart")


def test_b3_table_col_widths(b3_pptx_bytes):
    """``a:tblGrid`` 列宽(EMU)落进 table dict 的 ``col_widths``。"""
    table = [s for s in _b3_shapes(b3_pptx_bytes) if s["kind"] == "table"][0]
    assert table["col_widths"] == [3886200, 1234567]
    # 既有表格路径不受影响。
    assert table["rows"][0]["text"] == ["A1", "B1"]


def test_b3_legacy_table_has_empty_col_widths(minimal_pptx_bytes):
    """无 ``tblGrid`` 的表格 ``col_widths`` 为空列表(而非缺键)。"""
    pres = pptspine.open_bytes(minimal_pptx_bytes)
    table = [s for s in pres.slides()[0].shapes() if s["kind"] == "table"][0]
    assert table["col_widths"] == []
