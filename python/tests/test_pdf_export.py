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


def test_line_ends_draw_filled_heads_at_both_ends_and_shorten_body(
    arrow_connector_pptx: tuple[bytes, bytes],
) -> None:
    """线端装饰:头 stealth lg(10×10 pt)、尾 triangle med(6×6 pt,线宽 2 pt);
    两个线色实心多边形分落两端,线身缩到 stealth 凹点(72+6)与三角底(360−6)。"""
    arrowed, plain = arrow_connector_pptx
    pres = pptspine.open_bytes(arrowed)
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        pdf = pres.to_pdf()
    assert not caught, [str(w.message) for w in caught]
    drawings = _open_pdf(pdf)[0].get_drawings()
    heads = [d for d in drawings if d["fill"] == (1.0, 0.0, 0.0) and d["color"] is None]
    lines = [d for d in drawings if d["type"] == "s"]
    assert len(heads) == 2 and len(lines) == 1
    head, tail = sorted(heads, key=lambda d: d["rect"].x0)
    assert tuple(head["rect"]) == pytest.approx((72.0, 67.0, 82.0, 77.0), abs=0.01)
    assert len(head["items"]) == 4, "stealth = 4-point polygon"
    assert tuple(tail["rect"]) == pytest.approx((354.0, 69.0, 360.0, 75.0), abs=0.01)
    assert len(tail["items"]) == 3, "triangle = 3-point polygon"
    assert tuple(lines[0]["rect"]) == pytest.approx((78.0, 72.0, 354.0, 72.0), abs=0.01)

    # type="none" 两端:与无线端一致,整线到端点、无额外多边形。
    plain_drawings = _open_pdf(_export(plain)[0])[0].get_drawings()
    assert len(plain_drawings) == 1
    assert tuple(plain_drawings[0]["rect"]) == pytest.approx((72.0, 72.0, 360.0, 72.0), abs=0.01)


def test_custom_geometry_is_drawn_as_its_real_path_without_warning(
    custom_geometry_pptx_bytes: bytes,
) -> None:
    """custGeom 求值成真实路径:直接填充与仅 style fillRef 着色的 freeform 都画三角形(非方块),无近似告警。"""
    pres = pptspine.open_bytes(custom_geometry_pptx_bytes)
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        pdf = pres.to_pdf()
    msgs = [str(w.message) for w in caught]
    assert not [m for m in msgs if "custom-geometry-approximated" in m], msgs
    drawings = sorted(
        _open_pdf(pdf)[0].get_drawings(), key=lambda d: tuple(d["fill"])
    )
    assert [tuple(d["fill"]) for d in drawings] == [(0.0, 0.0, 1.0), (0.0, 1.0, 0.0)]
    assert tuple(drawings[0]["rect"]) == pytest.approx((72.0, 72.0, 172.0, 172.0), abs=0.01)
    assert tuple(drawings[1]["rect"]) == pytest.approx((300.0, 72.0, 400.0, 172.0), abs=0.01)
    for d in drawings:
        # 三角形 (0,0)-(10,10)-(0,10):不是四点方块;闭合边不进 items 时为 2 条线。
        lines = [i for i in d["items"] if i[0] == "l"]
        assert len(lines) in (2, 3), d["items"]
        assert not any(i[0] == "re" for i in d["items"]), d["items"]


def test_custom_geometry_unresolvable_guide_degrades_to_bbox_with_one_warning(
    unresolvable_custom_geometry_pptx_bytes: bytes,
) -> None:
    """路径引用未定义的参考线名 → 退回包围盒 + 原告警(一次),不抛错。"""
    pptx = unresolvable_custom_geometry_pptx_bytes
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        pdf = pptspine.open_bytes(pptx).to_pdf()
    msgs = [str(w.message) for w in caught]
    assert len([m for m in msgs if "custom-geometry-approximated" in m]) == 1, msgs
    (d,) = _open_pdf(pdf)[0].get_drawings()
    assert tuple(d["rect"]) == pytest.approx((72.0, 72.0, 172.0, 172.0), abs=0.01)


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


def test_b7_table_without_tblgrid_still_renders(minimal_pptx_bytes: bytes) -> None:
    """minimal deck 的 2x2 表格无 ``a:tblGrid``:等分列宽降级,单元格文字仍可提取。"""
    pdf, _ = _export(minimal_pptx_bytes)
    text = _open_pdf(pdf)[0].get_text()
    for cell in ("A1", "B1", "A2", "B2"):
        assert cell in text, f"table cell {cell!r} lost without tblGrid: {text!r}"


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


# --- run 级上下标 / 字符间距(§3.h:a:rPr@baseline / @spc)------------------------


def _spans(pdf: bytes) -> dict[str, dict]:
    """首页 span 按去空白文本索引(``origin`` = 基线起点,PDF 页坐标 y 向下)。"""
    out: dict[str, dict] = {}
    for block in _open_pdf(pdf)[0].get_text("dict")["blocks"]:
        for line in block.get("lines", []):
            for span in line["spans"]:
                key = span["text"].strip()
                if key:
                    out[key] = span
    return out


def test_baseline_script_offsets_follow_document_value(script_pptx_bytes: bytes) -> None:
    """上标 ``baseline=30000`` → 基线上抬 30% × 20pt = 6pt;下标 -25% → 下沉 5pt(≤1pt);
    字形缩小(×0.65);读回文本顺序不变。"""
    pdf, _ = _export(script_pptx_bytes)
    spans = _spans(pdf)
    base_y = spans["Base"]["origin"][1]
    assert abs(spans["mid"]["origin"][1] - base_y) <= 0.01, "基线文字同一基线"
    sup_y = spans["Sup"]["origin"][1]
    sub_y = spans["Sub"]["origin"][1]
    assert sup_y < base_y, "上标高于基线(PDF 读回 y 向下)"
    assert abs((base_y - sup_y) - 6.0) <= 1.0, f"上标偏移 {base_y - sup_y:.2f}pt ≠ 6pt"
    assert abs((sub_y - base_y) - 5.0) <= 1.0, f"下标偏移 {sub_y - base_y:.2f}pt ≠ 5pt"
    assert spans["Sup"]["size"] < spans["Base"]["size"] * 0.8, "上标字形缩小"
    words = [w[4] for w in _open_pdf(pdf)[0].get_text("words")]
    assert words == ["Base", "Sup", "mid", "Sub"], words


def _word_widths(pdf: bytes) -> dict[str, float]:
    return {w[4]: w[2] - w[0] for w in _open_pdf(pdf)[0].get_text("words")}


def test_char_spacing_positive_and_negative(
    char_spacing_pptx: tuple[bytes, bytes, bytes],
) -> None:
    """``spc=200``(lstStyle 继承)每字加宽 2pt、``spc=-100`` 每字收紧 1pt:词宽差 ≈ 间距 ×
    (字数 - 1)(≤1pt);读回词序不变。"""
    plain_pdf, wide_pdf, tight_pdf = (_export(b)[0] for b in char_spacing_pptx)
    plain, wide, tight = (_word_widths(p) for p in (plain_pdf, wide_pdf, tight_pdf))
    for word in ("Spacing", "test", "order"):
        gaps = len(word) - 1
        assert abs((wide[word] - plain[word]) - 2.0 * gaps) <= 1.0, (word, wide, plain)
        assert abs((plain[word] - tight[word]) - 1.0 * gaps) <= 1.0, (word, tight, plain)
    for pdf in (plain_pdf, wide_pdf, tight_pdf):
        words = [w[4] for w in _open_pdf(pdf)[0].get_text("words")]
        assert words == ["Spacing", "test", "order"], words


def test_caps_render_uppercase_and_small_caps_warns_once(caps_pptx_bytes: bytes) -> None:
    """``cap=all`` / ``cap=small`` 渲染为大写(small 近似),``small-caps`` 降级告警恰 1 条。"""
    pres = pptspine.open_bytes(caps_pptx_bytes)
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        pdf = pres.to_pdf()
    words = [w[4] for w in _open_pdf(pdf)[0].get_text("words")]
    assert words == ["ALL", "CAPS", "SMALL", "CAPS", "plain"], words
    assert len([w for w in caught if "small-caps" in str(w.message)]) == 1


# --- master / layout 非占位符形状继承(logo / 装饰条 / 页脚线;showMasterSp)--------

_BLUE, _RED, _GREEN = (0.0, 0.0, 1.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0)


def _paint_summary(pdf: bytes) -> tuple[list[tuple[str, tuple]], list, str]:
    """``(drawings, images, text)``:drawings 按绘制顺序取 ``(fill|stroke, 颜色)``。"""
    page = _open_pdf(pdf)[0]
    drawings = [
        ("fill", tuple(d["fill"])) if d["fill"] is not None else ("stroke", tuple(d["color"]))
        for d in page.get_drawings()
    ]
    return drawings, page.get_image_info(), page.get_text()


def test_master_layout_graphics_drawn_below_slide_content(
    master_graphics_pptx: tuple[bytes, bytes, bytes],
) -> None:
    """master logo 图片 + 蓝条、layout 红线都画出,且绘制顺序 master → layout → slide。"""
    pdf, _ = _export(master_graphics_pptx[0])
    drawings, images, text = _paint_summary(pdf)
    assert drawings == [("fill", _BLUE), ("stroke", _RED), ("fill", _GREEN)], drawings
    assert len(images) == 1, "master logo 经 master rels 解析并绘制"
    assert tuple(images[0]["bbox"]) == pytest.approx((600.0, 30.0, 700.0, 80.0), abs=0.01)
    # 内容流顺序:logo(Do)→ 蓝条 → 红线 → slide 绿框(z 序在正文 / slide 形状之下)。
    pos = [pdf.index(op) for op in (b" Do", b"0 0 1 rg", b"1 0 0 RG", b"0 1 0 rg")]
    assert pos == sorted(pos), pos
    assert "Slide body text" in text and "ACME Confidential" in text


def test_master_placeholders_are_not_drawn(
    master_graphics_pptx: tuple[bytes, bytes, bytes],
) -> None:
    """master / layout 上的空占位符只是模板:提示文字不出现在 PDF。"""
    pdf, _ = _export(master_graphics_pptx[0])
    text = _paint_summary(pdf)[2]
    for prompt in ("Master title prompt", "Master body prompt", "Layout body prompt"):
        assert prompt not in text


def test_master_text_styled_by_master_other_style(
    master_graphics_pptx: tuple[bytes, bytes, bytes],
) -> None:
    """master 页脚文字无直接格式 → 字号 11pt、颜色 7F007F 来自 master ``otherStyle``。"""
    pdf, _ = _export(master_graphics_pptx[0])
    span = _spans(pdf)["ACME Confidential"]
    assert span["size"] == pytest.approx(11.0, abs=0.01)
    assert span["color"] == 0x7F007F


def test_slide_show_master_sp_false_hides_master_and_layout_graphics(
    master_graphics_pptx: tuple[bytes, bytes, bytes],
) -> None:
    """slide ``showMasterSp="0"``(隐藏背景图形)→ master 与 layout 图形都不画。"""
    drawings, images, text = _paint_summary(_export(master_graphics_pptx[1])[0])
    assert drawings == [("fill", _GREEN)], drawings
    assert images == []
    assert "ACME Confidential" not in text and "Slide body text" in text


def test_layout_show_master_sp_false_hides_master_graphics_only(
    master_graphics_pptx: tuple[bytes, bytes, bytes],
) -> None:
    """layout ``showMasterSp="0"`` → 只隐藏 master 图形;layout 红线照画。"""
    drawings, images, text = _paint_summary(_export(master_graphics_pptx[2])[0])
    assert drawings == [("stroke", _RED), ("fill", _GREEN)], drawings
    assert images == []
    assert "ACME Confidential" not in text


def test_inherited_graphics_stay_out_of_shapes_and_text(
    master_graphics_pptx: tuple[bytes, bytes, bytes],
) -> None:
    """``Slide.shapes()`` / ``to_text`` / ``to_markdown`` 只含 slide 自身内容(API 不变)。"""
    pres = pptspine.open_bytes(master_graphics_pptx[0])
    assert len(pres.slides()[0].shapes()) == 2
    assert "ACME Confidential" not in pres.to_text()
    assert "ACME Confidential" not in pres.to_markdown()


def test_master_picture_background_resolves_through_master_rels(
    master_picture_bg_pptx_bytes: bytes,
) -> None:
    """master 图片背景的 ``r:embed`` 经 master 自身 rels 解析(此前 master / layout 部件
    不读 rels,图片背景静默丢失)→ 满页图片。"""
    images = _paint_summary(_export(master_picture_bg_pptx_bytes)[0])[1]
    assert [tuple(i["bbox"]) for i in images] == [pytest.approx((0.0, 0.0, 720.0, 540.0))]


# --- 超链接:run 级外链 → PDF URI 链接注释 --------------------------------------------


def test_run_hyperlink_exports_uri_link_annotation(semantic_pptx_bytes: bytes) -> None:
    """semantic deck 第 1 页:run 级外链 → 一个落在 run 排版包围盒内的 URI 注释;页内跳转 run
    与形状级(图片)链接本版不出注释;其余页(隐藏页不导出)无注释。"""
    pdf, pres = _export(semantic_pptx_bytes)
    doc = _open_pdf(pdf)
    links = doc[0].get_links()
    uris = [lk.get("uri") for lk in links]
    assert uris == ["https://example.com/home"], uris
    # 矩形与该 run 文字("our site")的词框:水平对齐(1 pt 容差),垂直至少重叠一半(引擎取
    # 行内 ink 范围,与 get_text_words 的行盒上下沿略有差异)。
    x0, y0, x1, y1 = links[0]["from"]
    words = [w for w in doc[0].get_text_words() if w[4] in {"our", "site"}]
    assert words, "找不到链接文字"
    assert abs(x0 - min(w[0] for w in words)) <= 1.0
    assert abs(x1 - max(w[2] for w in words)) <= 1.0
    wy0, wy1 = min(w[1] for w in words), max(w[3] for w in words)
    assert min(y1, wy1) - max(y0, wy0) >= 0.5 * (wy1 - wy0)
    assert all(not doc[i].get_links() for i in range(1, len(doc)))
