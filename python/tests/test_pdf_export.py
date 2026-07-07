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


# --- B-10:背景 layout/master 继承端到端 --------------------------------------


def test_b10_master_background_inherited_fills_full_page(
    master_background_pptx_bytes: bytes,
) -> None:
    """slide / layout 均无 `p:bg` → 继承 slideMaster 的纯色背景,导出 PDF 满页填充。"""
    pdf, _ = _export(master_background_pptx_bytes)
    assert b"0 1 0 rg" in pdf, "master background green fill missing"


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


# --- B-4:形状变换(rot/flip/avLst/prstDash/srcRect)------------------------------


def _words_bbox(words) -> tuple[float, float, float, float]:
    return (
        min(w[0] for w in words),
        min(w[1] for w in words),
        max(w[2] for w in words),
        max(w[3] for w in words),
    )


def test_b4_rotated_textbox_word_center_at_rect_center(
    rotated_textbox_pptx: tuple[bytes, bytes, tuple[int, int, int, int]],
) -> None:
    """旋转 45° 文本框:词 bbox 中心距矩形中心 ≤ 1 pt(旋转绕盒心,不漂移)。"""
    rot_pptx, plain_pptx, (ex, ey, ew, eh) = rotated_textbox_pptx
    cx = (ex + ew / 2) / EMU_PER_PT
    cy = (ey + eh / 2) / EMU_PER_PT

    for pptx in (plain_pptx, rot_pptx):
        pdf, _ = _export(pptx)
        words = _open_pdf(pdf)[0].get_text("words")
        assert words, "expected extractable words"
        x0, y0, x1, y1 = _words_bbox(words)
        assert (x0 + x1) / 2 == pytest.approx(cx, abs=1.0)
        assert (y0 + y1) / 2 == pytest.approx(cy, abs=1.0)

    # 旋转确实发生:45° 后词 bbox 的宽高都超过未旋转版(对角线铺开)。
    plain_pdf, _ = _export(plain_pptx)
    rot_pdf, _ = _export(rot_pptx)
    pw = _words_bbox(_open_pdf(plain_pdf)[0].get_text("words"))
    rw = _words_bbox(_open_pdf(rot_pdf)[0].get_text("words"))
    assert (rw[3] - rw[1]) > (pw[3] - pw[1]) + 5.0, "rotated words must span taller"


def test_b4_round_rect_adjust_changes_raster(
    round_rect_adjust_pptx: tuple[bytes, bytes],
) -> None:
    """roundRect avLst:adj=50000 与缺省的光栅不同(SSIM < 1.0),且都非空白。"""
    adjusted, default = round_rect_adjust_pptx
    pix_a = _open_pdf(_export(adjusted)[0])[0].get_pixmap()
    pix_d = _open_pdf(_export(default)[0])[0].get_pixmap()
    assert not _near_blank(pix_a)
    assert not _near_blank(pix_d)
    assert bytes(pix_a.samples) != bytes(pix_d.samples), "avLst adj must change the raster"


def test_b4_flip_h_moves_ink_to_the_other_side(
    flipped_triangle_pptx: tuple[bytes, bytes],
) -> None:
    """rtTriangle flipH:光栅非对称——直角边(墨量重心)在**形状矩形内**换边。"""
    flipped, plain = flipped_triangle_pptx
    # 形状矩形(pt = 72 dpi 光栅像素):off(914400,914400) ext(3657600,2743200)。
    rx0, rx1 = 72, 72 + 288
    mid = (rx0 + rx1) // 2

    def ink_halves(pix) -> tuple[float, float]:
        samples = pix.samples
        n, w, h = pix.n, pix.width, pix.height
        left = right = 0.0
        for row in range(h):
            base = row * w * n
            for col in range(rx0, min(rx1, w)):
                i = base + col * n
                dark = 255.0 - sum(samples[i + c] for c in range(min(n, 3))) / min(n, 3)
                if col < mid:
                    left += dark
                else:
                    right += dark
        return left, right

    pl, pr = ink_halves(_open_pdf(_export(plain)[0])[0].get_pixmap())
    fl, fr = ink_halves(_open_pdf(_export(flipped)[0])[0].get_pixmap())
    assert pl > pr, "unflipped rtTriangle is left-heavy inside its rect"
    assert fr > fl, "flipH must move the mass to the right half of the rect"


def test_b4_dash_pattern_emitted(dashed_connector_pptx: bytes) -> None:
    """prstDash="dash"、线宽 2 pt → 内容流(未压缩)出现 `[8 6] 0 d`。"""
    pdf, _ = _export(dashed_connector_pptx)
    assert b"[8 6] 0 d" in pdf, "expected DrawingML dash pattern (4/3 line widths)"


def test_b4_src_rect_crop_changes_raster_and_image_survives(
    src_rect_pptx: tuple[bytes, bytes],
) -> None:
    """srcRect 裁剪:图片存活(embed 恰一份)、光栅与不裁剪版不同、带剪裁路径。"""
    cropped, plain = src_rect_pptx
    pdf_c, _ = _export(cropped)
    pdf_p, _ = _export(plain)
    page_c = _open_pdf(pdf_c)[0]
    page_p = _open_pdf(pdf_p)[0]
    assert len(page_c.get_images(full=True)) == 1
    assert len(page_p.get_images(full=True)) == 1
    assert not _near_blank(page_c.get_pixmap())
    assert bytes(page_c.get_pixmap().samples) != bytes(page_p.get_pixmap().samples)
    assert b"W n" in pdf_c, "srcRect crop must clip the enlarged placement"


# --- B-5:Group 仿射(chOff/chExt 重映射 + 嵌套)----------------------------------


def _assert_words_match(pdf_a: bytes, pdf_b: bytes) -> None:
    """两份 PDF 的 get_text_words 逐词坐标一致(1 pt 门)、文本一致。"""
    wa = _open_pdf(pdf_a)[0].get_text("words")
    wb = _open_pdf(pdf_b)[0].get_text("words")
    assert wa and len(wa) == len(wb)
    for a, b in zip(wa, wb):
        assert a[4] == b[4], f"word text mismatch: {a[4]!r} vs {b[4]!r}"
        for i in range(4):
            assert a[i] == pytest.approx(b[i], abs=1.0), f"coord {i} of {a[4]!r}"


def test_b5_grouped_scaled_textbox_matches_flattened_twin(
    grouped_textbox_pptx: tuple[bytes, bytes],
) -> None:
    grouped, twin = grouped_textbox_pptx
    _assert_words_match(_export(grouped)[0], _export(twin)[0])


def test_b5_nested_groups_match_flattened_twin(
    nested_group_pptx: tuple[bytes, bytes],
) -> None:
    nested, twin = nested_group_pptx
    _assert_words_match(_export(nested)[0], _export(twin)[0])


# --- B-6:文本框锚定 / 行距 / 项目符号 --------------------------------------------

# Liberation Sans 度量(font units,em=2048):ascent 1854、descent 434、hhea lineGap 67。
_ASCENT, _DESCENT, _LINE_GAP, _EM = 1854, 434, 67, 2048


def test_b6_bottom_anchor_content_bottom_aligned(
    anchored_textbox_pptx: tuple[bytes, bytes, tuple[int, int, int, int]],
) -> None:
    """底锚:内容块底(行盒降部线)贴内容矩形底 ≤1pt;基线 = 内容底 − descent。

    实测:get_text_words 的 bbox 底 y1 = 字体行盒降部线(与实际字形无关、恒定),
    引擎把该行盒底对齐到内容矩形底(rect.bottom − bIns),误差 0。故最后一行**基线**
    落在内容底上方 descent 处(descent = 434/2048×size)。
    """
    bottom_pptx, top_pptx, (ex, ey, ew, eh) = anchored_textbox_pptx
    size = 20.0
    descent = _DESCENT / _EM * size  # 4.238 pt @20pt
    content_bottom = (ey + eh) / EMU_PER_PT - INSET_TB  # rect.bottom − bIns = 212.4

    pdf_b, _ = _export(bottom_pptx)
    words_b = _open_pdf(pdf_b)[0].get_text("words")
    assert words_b, "expected extractable words"
    y1_b = max(w[3] for w in words_b)
    # 主断言:行盒底(降部线)对齐内容底 ≤1pt。
    assert y1_b == pytest.approx(content_bottom, abs=1.0)
    # 度量派生:最后一行基线在内容底上方 descent 处。
    baseline_b = y1_b - descent
    assert baseline_b == pytest.approx(content_bottom - descent, abs=1.0)

    # 方向性:底锚词顶明显低于同尺寸顶锚孪生(位移 ≈ 盒内高 − 行高 量级)。
    pdf_t, _ = _export(top_pptx)
    words_t = _open_pdf(pdf_t)[0].get_text("words")
    assert words_t, "expected extractable words"
    y0_b = min(w[1] for w in words_b)
    y0_t = min(w[1] for w in words_t)
    line_h = (_ASCENT + _DESCENT + _LINE_GAP) / _EM * size  # 行盒高(含 lineGap ≈ 23 pt)
    box_interior = eh / EMU_PER_PT - 2 * INSET_TB           # 136.8 pt
    assert y0_b - y0_t > box_interior - 2 * line_h, "bottom anchor must drop the line by ~box height"


def test_b6_line_spacing_doubles_gap(line_spacing_pptx: tuple[bytes, bytes]) -> None:
    """lnSpc 200% 的行间词 y 差 ≈ 2× 100%(±5%)。"""
    pptx_100, pptx_200 = line_spacing_pptx

    def line_gap(pptx: bytes) -> float:
        words = _open_pdf(_export(pptx)[0])[0].get_text("words")
        y = {w[4]: w[1] for w in words}
        assert "Alpha" in y and "Beta" in y, f"missing line words: {sorted(y)}"
        return y["Beta"] - y["Alpha"]

    delta100 = line_gap(pptx_100)
    delta200 = line_gap(pptx_200)
    assert delta100 > 0
    assert delta200 == pytest.approx(2 * delta100, rel=0.05)


def test_b6_bullet_and_autonum_readback(bulleted_textbox_pptx_bytes: bytes) -> None:
    """buChar '•' 与 buAutoNum arabicPeriod '1.' 均作为可提取文本落在读回里。"""
    pdf, _ = _export(bulleted_textbox_pptx_bytes)
    text = _open_pdf(pdf)[0].get_text()
    assert "•" in text, f"buChar bullet missing: {text!r}"
    assert "1." in text, f"buAutoNum arabicPeriod first number '1.' missing: {text!r}"


# --- B-7:表格几何 / 网格边框 / 合并格 --------------------------------------------


def _line_segments(page) -> tuple[list, list]:
    """从 ``get_drawings()`` 收集水平 / 竖直线段:``(horizontals, verticals)``。

    每条归一为 ``(const_coord, lo, hi)``:水平线 = ``(y, x_lo, x_hi)``、竖直线 = ``(x, y_lo, y_hi)``。
    """
    horizontals: list = []
    verticals: list = []
    for d in page.get_drawings():
        for it in d.get("items", []):
            if it[0] != "l":
                continue
            p1, p2 = it[1], it[2]
            if abs(p1.y - p2.y) < 0.5:
                horizontals.append((p1.y, min(p1.x, p2.x), max(p1.x, p2.x)))
            elif abs(p1.x - p2.x) < 0.5:
                verticals.append((p1.x, min(p1.y, p2.y), max(p1.y, p2.y)))
    return horizontals, verticals


def test_b7_cell_first_word_x_within_1pt(
    grid_table_pptx: tuple[bytes, tuple[int, int], tuple[int, ...], int, tuple[str, ...]],
) -> None:
    """每格首词 x0 = off_x + 累积列宽 + mar_l(缺省 91440 EMU=7.2pt),逐格 ≤1pt 门。"""
    pptx, (off_x, _off_y), col_widths, _cy, cell_texts = grid_table_pptx
    pdf, _ = _export(pptx)
    words = _open_pdf(pdf)[0].get_text("words")
    first_x = {w[4]: w[0] for w in words}

    off_x_pt = off_x / EMU_PER_PT
    cum = 0
    for i, text in enumerate(cell_texts):
        assert text in first_x, f"cell word {text!r} missing: {sorted(first_x)}"
        expected = off_x_pt + cum / EMU_PER_PT + INSET_LR  # INSET_LR == mar_l 缺省 7.2pt
        assert first_x[text] == pytest.approx(expected, abs=1.0), (
            f"cell {i} {text!r}: x0={first_x[text]} expected≈{expected}"
        )
        cum += col_widths[i]


def test_b7_grid_borders_visible(
    grid_table_pptx: tuple[bytes, tuple[int, int], tuple[int, ...], int, tuple[str, ...]],
) -> None:
    """tcBorders → get_drawings() 出现落在网格坐标上的红色边框线段(1–2pt 容差)。"""
    pptx, (off_x, off_y), col_widths, ext_cy, _texts = grid_table_pptx
    pdf, _ = _export(pptx)
    page = _open_pdf(pdf)[0]
    horizontals, verticals = _line_segments(page)
    assert horizontals and verticals, "expected border line segments"

    row_top = off_y / EMU_PER_PT                # 144
    row_bottom = (off_y + ext_cy) / EMU_PER_PT  # 202.4
    left_x = off_x / EMU_PER_PT                  # 首格左界 72
    boundary_x = left_x + col_widths[0] / EMU_PER_PT  # 首列右界 252

    # 首列右界处存在贯穿整行的竖直边框线。
    assert any(
        abs(x - boundary_x) <= 2.0 and lo <= row_top + 2.0 and hi >= row_bottom - 2.0
        for (x, lo, hi) in verticals
    ), f"no vertical border at col boundary x={boundary_x}: {sorted(set(verticals))}"

    # 行顶存在覆盖首格 x 区间的水平边框线。
    assert any(
        abs(y - row_top) <= 2.0 and lo <= left_x + 2.0 and hi >= boundary_x - 2.0
        for (y, lo, hi) in horizontals
    ), f"no horizontal border at row top y={row_top}: {sorted(set(horizontals))}"

    # 边框描边为红色(FF0000)。
    red = [
        d
        for d in page.get_drawings()
        if d.get("color") and tuple(round(c, 3) for c in d["color"]) == (1.0, 0.0, 0.0)
    ]
    assert red, "expected red-stroked border drawings"


def test_b7_merged_cell_has_no_internal_border(
    merged_border_table_pptx: tuple[bytes, bytes, tuple[int, int], tuple[int, ...], int],
) -> None:
    """gridSpan=2 跨列格:内部列边界无竖直边框(仅外框);不合并双列行则有内部竖线。"""
    merged_pptx, plain_pptx, (off_x, off_y), col_widths, ext_cy = merged_border_table_pptx
    row_top = off_y / EMU_PER_PT
    row_bottom = (off_y + ext_cy) / EMU_PER_PT
    left_x = off_x / EMU_PER_PT                                 # 72
    internal_x = left_x + col_widths[0] / EMU_PER_PT            # 内部列边界 252
    right_x = left_x + sum(col_widths) / EMU_PER_PT             # 外框右界 342

    def verticals_at(pptx: bytes, x_target: float) -> list:
        _, verticals = _line_segments(_open_pdf(_export(pptx)[0])[0])
        return [
            (x, lo, hi)
            for (x, lo, hi) in verticals
            if abs(x - x_target) <= 1.5 and lo <= row_top + 2.0 and hi >= row_bottom - 2.0
        ]

    # 合并格:内部列边界无竖线,但外框(左 / 右)仍有竖线。
    assert not verticals_at(merged_pptx, internal_x), "merged span must omit the inner divider"
    assert verticals_at(merged_pptx, left_x), "merged span keeps its outer left border"
    assert verticals_at(merged_pptx, right_x), "merged span keeps its outer right border"

    # 反衬:同尺寸不合并双列行在同一内部边界处**有**竖直分隔线。
    assert verticals_at(plain_pptx, internal_x), "unmerged row must draw the inner divider"


# --- 纵排告警(Task 4 / PRD §6:bodyPr@vert 水平降级,逐种类一次)------------------


def test_vertical_text_warns_once_and_stays_horizontal(
    vertical_text_pptx_bytes: bytes,
) -> None:
    """两个 bodyPr@vert 纵排框:'vertical-text' 告警恰 1 条;文字仍水平可提取(降级非丢弃)。"""
    pres = pptspine.open_bytes(vertical_text_pptx_bytes)
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        pdf = pres.to_pdf()
    assert pdf.startswith(b"%PDF-")
    vertical_warnings = [w for w in caught if "vertical-text" in str(w.message)]
    # 两个纵排框同种类只上浮一次。
    assert len(vertical_warnings) == 1
    # 水平降级:文字仍以水平行可提取(未丢弃)。
    text = _open_pdf(pdf)[0].get_text()
    assert "Vertical one" in text
    assert "Vertical two" in text
