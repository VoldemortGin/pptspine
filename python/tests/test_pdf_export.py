"""PDF 导出验收(PRD-PDF-EXPORT §8 B-1 / B-2 绿条)。

读回栈用 pip 安装的 ``pdfspine``(PyMuPDF 兼容 API):页数 / 页面尺寸(B-1)、
``get_text_words`` 坐标 1 pt 门 + token-F1 / order ≥ 0.99 + 光栅非空白 +
单 FontFile2(B-2)。评分数学复刻 pdfspine ``conformance/gt/score.py`` 的
``content_scores`` / ``order_score``(multiset 交 + SequenceMatcher 对齐),
空白判定复刻 ``render_diff.py::_near_blank``(灰度方差 < 4 视为空白)。
"""

from __future__ import annotations

import warnings
from collections import Counter
from difflib import SequenceMatcher

import pdfspine
import pytest

import pptspine

# OOXML bodyPr 缺省内边距(pt):左右 91440 EMU、上下 45720 EMU。
INSET_LR = 7.2
INSET_TB = 3.6
EMU_PER_PT = 12700.0


# --- 评分辅助(复刻 pdfspine conformance/gt/score.py 的数学)---------------------


def _tokenize(text: str) -> list[str]:
    toks: list[str] = []
    buf: list[str] = []
    for ch in text:
        if ch.isspace():
            if buf:
                toks.append("".join(buf))
                buf = []
        elif "一" <= ch <= "鿿":
            if buf:
                toks.append("".join(buf))
                buf = []
            toks.append(ch)
        else:
            buf.append(ch)
    if buf:
        toks.append("".join(buf))
    return toks


def _token_f1(hyp: str, ref: str) -> float:
    ht, rt = _tokenize(hyp), _tokenize(ref)
    if not ht and not rt:
        return 1.0
    if not ht or not rt:
        return 0.0
    overlap = sum((Counter(ht) & Counter(rt)).values())
    precision = overlap / len(ht)
    recall = overlap / len(rt)
    if precision + recall == 0:
        return 0.0
    return 2 * precision * recall / (precision + recall)


def _order_score(hyp: str, ref: str) -> float:
    ht, rt = _tokenize(hyp), _tokenize(ref)
    if not ht or not rt:
        return 1.0
    shared = sum((Counter(ht) & Counter(rt)).values())
    if shared == 0:
        return 1.0
    matched = sum(m.size for m in SequenceMatcher(None, ht, rt).get_matching_blocks())
    return matched / shared


def _ref_text_without_separators(pres) -> str:
    """``to_text()`` 去掉 ``--- slide N ---`` 分隔行与 Notes 块(fixture 无备注)。"""
    lines = [
        line
        for line in pres.to_text().splitlines()
        if not (line.startswith("--- slide ") and line.endswith(" ---"))
    ]
    return "\n".join(lines)


def _near_blank(pix) -> bool:
    """复刻 render_diff.py::_near_blank:灰度方差 < 4(std < 2 灰阶)即空白。"""
    samples = pix.samples
    n = pix.n
    grays = [
        sum(samples[i + c] for c in range(min(n, 3))) / min(n, 3)
        for i in range(0, len(samples), n)
    ]
    mu = sum(grays) / len(grays)
    var = sum((g - mu) ** 2 for g in grays) / len(grays)
    return var < 4.0


def _open_pdf(pdf: bytes):
    return pdfspine.open(stream=pdf, filetype="pdf")


def _export(pptx_bytes: bytes) -> tuple[bytes, object]:
    pres = pptspine.open_bytes(pptx_bytes)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        pdf = pres.to_pdf()
    return pdf, pres


# --- B-1:空白页装配 + Python 接线 ------------------------------------------------


def test_b1_pdf_bytes_nonempty_and_magic(minimal_pptx_bytes: bytes) -> None:
    pdf, _ = _export(minimal_pptx_bytes)
    assert isinstance(pdf, bytes)
    assert len(pdf) > 0
    assert pdf.startswith(b"%PDF-")


def test_b1_page_count_and_rect_4x3(minimal_pptx_bytes: bytes) -> None:
    pdf, pres = _export(minimal_pptx_bytes)
    doc = _open_pdf(pdf)
    assert doc.page_count == pres.slide_count == 1
    w, h = pres.slide_size_points
    assert (w, h) == (720.0, 540.0)
    for page in doc:
        assert tuple(page.rect) == pytest.approx((0.0, 0.0, w, h))


def test_b1_page_count_and_rect_16x9(widescreen_pptx_bytes: bytes) -> None:
    pdf, pres = _export(widescreen_pptx_bytes)
    doc = _open_pdf(pdf)
    assert doc.page_count == pres.slide_count == 2
    w, h = pres.slide_size_points
    assert (w, h) == (960.0, 540.0)
    for page in doc:
        assert tuple(page.rect) == pytest.approx((0.0, 0.0, w, h))


def test_b1_save_pdf_writes_file(minimal_pptx_bytes: bytes, tmp_path) -> None:
    pres = pptspine.open_bytes(minimal_pptx_bytes)
    out = tmp_path / "deck.pdf"
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        pres.save_pdf(out)
    data = out.read_bytes()
    assert data.startswith(b"%PDF-")
    assert _open_pdf(data).page_count == 1


def test_b1_font_map_kwarg_smoke(minimal_pptx_bytes: bytes) -> None:
    pres = pptspine.open_bytes(minimal_pptx_bytes)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        pdf = pres.to_pdf(font_map={"Calibri": "Helvetica"})
    assert pdf.startswith(b"%PDF-")


# --- B-2:显式几何文本框 ---------------------------------------------------------


def test_b2_word_bbox_within_1pt(b2_textbox_pptx: tuple[bytes, tuple[int, int, int, int]]) -> None:
    pptx, (ex, ey, ew, eh) = b2_textbox_pptx
    pdf, _ = _export(pptx)
    page = _open_pdf(pdf)[0]
    words = page.get_text("words")
    assert words, "expected extractable words"

    content_left = ex / EMU_PER_PT + INSET_LR
    content_top = ey / EMU_PER_PT + INSET_TB
    content_right = (ex + ew) / EMU_PER_PT - INSET_LR
    content_bottom = (ey + eh) / EMU_PER_PT - INSET_TB

    x0 = min(w[0] for w in words)
    y0 = min(w[1] for w in words)
    x1 = max(w[2] for w in words)
    y1 = max(w[3] for w in words)

    # 左上角贴内容原点(左对齐、顶部锚定);整体 bbox 不越出内容矩形。1 pt 门限。
    assert x0 == pytest.approx(content_left, abs=1.0)
    assert y0 == pytest.approx(content_top, abs=1.0)
    assert x1 <= content_right + 1.0
    assert y1 <= content_bottom + 1.0


def test_b2_token_f1_and_order(b2_textbox_pptx: tuple[bytes, tuple[int, int, int, int]]) -> None:
    pptx, _ = b2_textbox_pptx
    pdf, pres = _export(pptx)
    doc = _open_pdf(pdf)
    hyp = "\n".join(page.get_text() for page in doc)
    ref = _ref_text_without_separators(pres)
    assert _token_f1(hyp, ref) >= 0.99
    assert _order_score(hyp, ref) >= 0.99


def test_b2_raster_not_near_blank(
    b2_textbox_pptx: tuple[bytes, tuple[int, int, int, int]],
) -> None:
    pptx, _ = b2_textbox_pptx
    pdf, _ = _export(pptx)
    pix = _open_pdf(pdf)[0].get_pixmap()
    assert not _near_blank(pix)


def test_b2_exactly_one_fontfile2_per_face(
    b2_textbox_pptx: tuple[bytes, tuple[int, int, int, int]],
) -> None:
    # 两个 run(bold / italic)各用一个 face:恰好两个子集化 FontFile2,绝无整库嵌入。
    pptx, _ = b2_textbox_pptx
    pdf, _ = _export(pptx)
    assert pdf.count(b"/FontFile2") == 2


def test_b2_single_face_single_fontfile2(widescreen_pptx_bytes: bytes) -> None:
    # 两张 slide 同字体同样式:全文档一个 face、一个 FontFile2。
    pdf, _ = _export(widescreen_pptx_bytes)
    assert pdf.count(b"/FontFile2") == 1


# --- 告警上浮(PRD §6:逐种类一次)-----------------------------------------------


def test_warnings_surface_once_per_kind(unknown_presets_pptx_bytes: bytes) -> None:
    pres = pptspine.open_bytes(unknown_presets_pptx_bytes)
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        pdf = pres.to_pdf()
    assert pdf.startswith(b"%PDF-")
    preset_warnings = [
        w for w in caught if "unsupported shape preset" in str(w.message)
    ]
    # deck 有 cloud + heart 两个未知预设,但同种类只上浮一次。
    assert len(preset_warnings) == 1


# --- 端到端综合 deck:继承链 + 列表 + 预设形状 + 图片 ------------------------------


def test_e2e_full_deck_roundtrip(e2e_pptx_bytes: bytes) -> None:
    pdf, pres = _export(e2e_pptx_bytes)
    doc = _open_pdf(pdf)
    assert doc.page_count == 1
    page = doc[0]
    text = page.get_text()

    # 标题占位符文字(几何 / 字号 / 颜色全走 layout→master→theme 链)。
    assert "Quarterly Review" in text
    # 正文列表逐条 + 继承的 bullet 字符(master bodyStyle lvl1 '•' / lvl2 '–')。
    assert "Revenue up strongly" in text
    assert "Costs held flat" in text
    assert "Cloud spend detail" in text
    assert "•" in text
    assert "–" in text
    # 预设形状上的文字。
    assert "GO TEAM" in text
    # 图片存活(embed 恰好一份)。
    assert len(page.get_images(full=True)) == 1
    # 版面非空白。
    assert not _near_blank(page.get_pixmap())


def test_e2e_title_geometry_from_master(e2e_pptx_bytes: bytes) -> None:
    """标题(slide 无 xfrm)的词坐标应落在 master 标题占位符矩形内(B-9 链回填)。"""
    pdf, _ = _export(e2e_pptx_bytes)
    page = _open_pdf(pdf)[0]
    title_words = [w for w in page.get_text("words") if w[4] in ("Quarterly", "Review")]
    assert title_words
    # master 标题占位符:off(838200,365125) ext(7772400,1325563) EMU。
    rx = 838_200 / EMU_PER_PT
    ry = 365_125 / EMU_PER_PT
    rw = 7_772_400 / EMU_PER_PT
    rh = 1_325_563 / EMU_PER_PT
    for w in title_words:
        assert rx - 1.0 <= w[0] and w[2] <= rx + rw + 1.0
        assert ry - 1.0 <= w[1] and w[3] <= ry + rh + 1.0
