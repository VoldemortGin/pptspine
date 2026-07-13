#!/usr/bin/env python3
"""自渲染 SSIM 基线门 —— PRD-PDF-EXPORT §8 B-11 门(4)。

**committed ``.ssimref`` refs @ ``--min-ssim 0.97`` 的 CI 门。**

与 LibreOffice oracle(``scripts/lo_oracle_ssim.py``,**local-only、never-CI**)不同:本门是
**自渲染回归门**——把每个合成 fixture 经 ``pptspine`` ``to_pdf()`` 渲染、``pdfspine`` 栅格化,
和 git 里 committed 的灰度参考(``python/tests/ssim_refs/*.ssimref``)逐页算 SSIM,要求 ≥ 阈值
(缺省 0.97),且尺寸一致、无「回归成空白」。自比对恒为 1.0,任何**无意**渲染漂移都会掉分。

- 参考在渲染稳定后一次性 ``--make-references`` 生成并入库(≤192px 灰度、uint8,git 友好、体积小)。
- 渲染**有意**变更后,同样用 ``--make-references`` 重生成——把「有意更新」与「无意漂移」分开。

SSIM / 灰度下采样数学**复刻** ``pdfspine`` ``conformance/gt/render_diff.py`` 的参考实现:family
风格是复刻数学(如 ``test_pdf_export.py`` 复刻 ``score.py``),而非在 CI 运行期依赖源码树里的
``render_diff.py`` —— 它随 pip 包**不分发**,CI 里只有 pip 安装的 ``pdfspine``。

字体环境敏感(PRD §9 风险 5):参考只在**固定字体的 runner** 上生成、门也只在其上强制。

用法(仓库根)::

    python scripts/ssim_baseline.py                    # 检查(门):当前渲染 vs committed 参考
    python scripts/ssim_baseline.py --make-references   # 有意重生成基线(渲染稳定/有意变更后)
    python scripts/ssim_baseline.py --min-ssim 0.97 --json out.json
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import math
import struct
import sys
import warnings
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
REF_DIR_DEFAULT = REPO_ROOT / "python" / "tests" / "ssim_refs"
DPI = 100.0
REF_MAX_DIM = 192
REF_MAGIC = b"PPTREF1\n"
MIN_SSIM_DEFAULT = 0.97


# --- 灰度下采样 + SSIM(复刻 pdfspine render_diff.py 的参考实现)-----------------


def _drop_alpha(samples: bytes, w: int, h: int) -> bytes:
    out = bytearray(w * h * 3)
    mv = memoryview(samples)
    j = 0
    for i in range(0, len(mv), 4):
        out[j] = mv[i]
        out[j + 1] = mv[i + 1]
        out[j + 2] = mv[i + 2]
        j += 3
    return bytes(out)


def _to_gray_downsampled(
    w: int, h: int, n: int, samples: bytes, max_dim: int = REF_MAX_DIM
) -> tuple[int, int, list[float]]:
    """把 RGB(/灰度)缓冲下采样成小的灰度浮点列表,返回 ``(gw, gh, pixels)``([0,255])。"""
    scale = max(1, math.ceil(max(w, h) / max_dim))
    gw = max(1, w // scale)
    gh = max(1, h // scale)
    mv = memoryview(samples)
    stride = w * n
    out = [0.0] * (gw * gh)
    if scale == 1:
        probes_y = (0,)
        probes_x = (0,)
    else:
        q = max(1, scale // 4)
        probes_y = (0, scale // 2, scale - 1, q, scale - 1 - q)
        probes_x = probes_y
    for gy in range(gh):
        y0 = gy * scale
        for gx in range(gw):
            x0 = gx * scale
            acc = 0
            cnt = 0
            for dy in probes_y:
                yy = y0 + dy
                if yy >= h:
                    continue
                rowbase = yy * stride
                for dx in probes_x:
                    xx = x0 + dx
                    if xx >= w:
                        continue
                    base = rowbase + xx * n
                    if n >= 3:
                        acc += 299 * mv[base] + 587 * mv[base + 1] + 114 * mv[base + 2]
                    else:
                        acc += 1000 * mv[base]
                    cnt += 1
            out[gy * gw + gx] = acc / (cnt * 1000) if cnt else 0.0
    return gw, gh, out


def _resize_gray(gw: int, gh: int, px: list[float], tw: int, th: int) -> list[float]:
    """灰度缓冲最近邻缩放到 ``(tw, th)``。"""
    if gw == tw and gh == th:
        return px
    out = [0.0] * (tw * th)
    for y in range(th):
        sy = min(gh - 1, int(y * gh / th))
        for x in range(tw):
            sx = min(gw - 1, int(x * gw / tw))
            out[y * tw + x] = px[sy * gw + sx]
    return out


def ssim(a: list[float], b: list[float], w: int, h: int, win: int = 7) -> float:
    """窗口化 SSIM(Wang et al. 2004),两张等尺寸灰度缓冲,返回全窗口均值。纯 Python。"""
    if w < win or h < win:
        win = min(w, h)
        if win < 2:
            return 1.0 if a == b else 0.0
    c1 = (0.01 * 255) ** 2
    c2 = (0.03 * 255) ** 2
    total = 0.0
    count = 0
    npx = win * win
    step = max(1, win // 2)
    for y0 in range(0, h - win + 1, step):
        for x0 in range(0, w - win + 1, step):
            sa = sb = saa = sbb = sab = 0.0
            for yy in range(y0, y0 + win):
                row = yy * w
                for xx in range(x0, x0 + win):
                    va = a[row + xx]
                    vb = b[row + xx]
                    sa += va
                    sb += vb
                    saa += va * va
                    sbb += vb * vb
                    sab += va * vb
            mu_a = sa / npx
            mu_b = sb / npx
            var_a = saa / npx - mu_a * mu_a
            var_b = sbb / npx - mu_b * mu_b
            cov = sab / npx - mu_a * mu_b
            num = (2 * mu_a * mu_b + c1) * (2 * cov + c2)
            den = (mu_a * mu_a + mu_b * mu_b + c1) * (var_a + var_b + c2)
            total += num / den if den else 1.0
            count += 1
    return total / count if count else 1.0


def _mean(px: list[float]) -> float:
    return sum(px) / len(px) if px else 0.0


def _near_blank(px: list[float]) -> bool:
    """灰度渲染近乎均匀场(方差 < 4,std < 2 灰阶)—— 常见的「画失败」征兆。"""
    if not px:
        return True
    mu = _mean(px)
    var = sum((v - mu) ** 2 for v in px) / len(px)
    return var < 4.0


# --- .ssimref 二进制格式(magic + JSON 头 + uint8 灰度体)-------------------------


def write_reference(path: Path, gw: int, gh: int, px: list[float], dpi: float) -> None:
    body = bytes(max(0, min(255, int(round(v)))) for v in px)
    header = json.dumps(
        {"gw": gw, "gh": gh, "dpi": dpi, "max_dim": REF_MAX_DIM, "mean_gray": round(_mean(px), 2)}
    ).encode("utf-8")
    with open(path, "wb") as fh:
        fh.write(REF_MAGIC)
        fh.write(struct.pack("<I", len(header)))
        fh.write(header)
        fh.write(body)


def read_reference(path: Path) -> tuple[int, int, list[float], dict]:
    with open(path, "rb") as fh:
        magic = fh.read(len(REF_MAGIC))
        if magic != REF_MAGIC:
            raise ValueError(f"{path}: bad reference magic {magic!r}")
        (hlen,) = struct.unpack("<I", fh.read(4))
        header = json.loads(fh.read(hlen).decode("utf-8"))
        body = fh.read()
    return header["gw"], header["gh"], [float(b) for b in body], header


# --- fixture 矩阵 + 渲染 + 栅格化 -------------------------------------------------


def _load_conftest():
    path = REPO_ROOT / "python" / "tests" / "conftest.py"
    spec = importlib.util.spec_from_file_location("conftest", path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot import conftest from {path}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def fixture_matrix() -> dict[str, bytes]:
    """全合成 fixture 矩阵物化成 ``名字 -> pptx 字节``(取每对 deck 的主变体)。"""
    conftest = _load_conftest()

    def fx(name: str):
        return getattr(conftest, name).__wrapped__()

    return {
        "minimal_4x3_text_table": fx("minimal_pptx_bytes"),
        "widescreen_16x9_2slides": fx("widescreen_pptx_bytes"),
        "b2_textbox": fx("b2_textbox_pptx")[0],
        "unknown_presets_cloud_heart": fx("unknown_presets_pptx_bytes"),
        "e2e_inheritance_chain": conftest.build_e2e_pptx(),
        "master_background": fx("master_background_pptx_bytes"),
        "rotated_textbox_45deg": fx("rotated_textbox_pptx")[0],
        "round_rect_adjust": fx("round_rect_adjust_pptx")[0],
        "flipped_triangle": fx("flipped_triangle_pptx")[0],
        "dashed_connector": fx("dashed_connector_pptx"),
        "src_rect_crop": fx("src_rect_pptx")[0],
        "grouped_textbox": fx("grouped_textbox_pptx")[0],
        "nested_group": fx("nested_group_pptx")[0],
        "anchored_textbox_bottom": fx("anchored_textbox_pptx")[0],
        "line_spacing_200pct": fx("line_spacing_pptx")[1],
        "bulleted_textbox": fx("bulleted_textbox_pptx_bytes"),
        "vertical_text_degraded": fx("vertical_text_pptx_bytes"),
    }


def _render_pdf(pptx_bytes: bytes) -> bytes:
    import pptspine

    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        return pptspine.open_bytes(pptx_bytes).to_pdf()


def _pdf_pages_gray(
    pdf_bytes: bytes, dpi: float = DPI, max_dim: int = REF_MAX_DIM
) -> list[tuple[int, int, list[float]]]:
    import pdfspine

    doc = pdfspine.open(stream=pdf_bytes, filetype="pdf")
    zoom = dpi / 72.0
    pages = []
    for page in doc:
        pm = page.get_pixmap(matrix=pdfspine.Matrix(zoom, zoom))
        samples = bytes(pm.samples)
        w, h, n = pm.width, pm.height, pm.n
        if n == 4:
            samples, n = _drop_alpha(samples, w, h), 3
        pages.append(_to_gray_downsampled(w, h, n, samples, max_dim=max_dim))
    return pages


def _render_matrix_gray(dpi: float = DPI) -> dict[str, list[tuple[int, int, list[float]]]]:
    return {name: _pdf_pages_gray(_render_pdf(b), dpi) for name, b in fixture_matrix().items()}


# --- 生成 / 检查 -----------------------------------------------------------------


def make_references(ref_dir: Path = REF_DIR_DEFAULT, dpi: float = DPI) -> int:
    ref_dir.mkdir(parents=True, exist_ok=True)
    for stale in ref_dir.glob("*.ssimref"):
        stale.unlink()
    count = 0
    for name, pages in _render_matrix_gray(dpi).items():
        for i, (gw, gh, px) in enumerate(pages):
            out = ref_dir / f"{name}__p{i}.ssimref"
            write_reference(out, gw, gh, px, dpi)
            print(f"wrote {out.relative_to(REPO_ROOT)} ({gw}x{gh}, mean_gray={round(_mean(px), 1)})")
            count += 1
    return count


def check_references(
    ref_dir: Path = REF_DIR_DEFAULT, dpi: float = DPI, min_ssim: float = MIN_SSIM_DEFAULT
) -> dict:
    """当前渲染 vs committed 参考,逐页 SSIM;返回带顶层 ``passed`` 的 payload。"""
    rendered = _render_matrix_gray(dpi)
    expected = {f"{name}__p{i}" for name, pages in rendered.items() for i in range(len(pages))}
    present = {p.stem for p in ref_dir.glob("*.ssimref")}

    docs: list[dict] = []
    all_ok = True

    for orphan in sorted(present - expected):
        name, _, page = orphan.rpartition("__p")
        docs.append(
            {
                "name": name,
                "page": int(page) if page.isdigit() else -1,
                "ssim": None,
                "passed": False,
                "error": "orphan reference (无对应渲染页;fixture 删了但参考没删?)",
            }
        )
        all_ok = False

    for name, pages in rendered.items():
        for i, (cgw, cgh, cpx) in enumerate(pages):
            rec: dict = {"name": name, "page": i, "ssim": None, "passed": False, "error": None}
            ref_path = ref_dir / f"{name}__p{i}.ssimref"
            if not ref_path.exists():
                rec["error"] = f"missing reference {ref_path.name}(跑 --make-references)"
                docs.append(rec)
                all_ok = False
                continue
            rgw, rgh, rpx, rhdr = read_reference(ref_path)
            tw, th = min(rgw, cgw), min(rgh, cgh)
            rfit = _resize_gray(rgw, rgh, rpx, tw, th)
            cfit = _resize_gray(cgw, cgh, cpx, tw, th)
            s = ssim(cfit, rfit, tw, th)
            rec["ssim"] = round(s, 4)
            rec["size_match"] = rgw == cgw and rgh == cgh
            rec["cur_mean_gray"] = round(_mean(cpx), 1)
            rec["ref_mean_gray"] = round(rhdr.get("mean_gray", _mean(rpx)), 1)
            blanked = _near_blank(cpx) and not _near_blank(rpx)
            rec["passed"] = (s >= min_ssim) and rec["size_match"] and not blanked
            if blanked:
                rec["error"] = "regressed to near-blank vs an inked reference"
            elif not rec["size_match"]:
                rec["error"] = f"raster size changed: ref {rgw}x{rgh}, cur {cgw}x{cgh}"
            elif s < min_ssim:
                rec["error"] = f"ssim {round(s, 4)} < {min_ssim}"
            docs.append(rec)
            all_ok = all_ok and rec["passed"]

    return {
        "mode": "self-render-no-oracle",
        "dpi": dpi,
        "min_ssim": min_ssim,
        "n": len(docs),
        "passed": all_ok,
        "docs": docs,
    }


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--ref-dir", default=str(REF_DIR_DEFAULT))
    parser.add_argument("--dpi", type=float, default=DPI)
    parser.add_argument("--min-ssim", type=float, default=MIN_SSIM_DEFAULT)
    parser.add_argument(
        "--make-references",
        action="store_true",
        help="有意重生成 committed 基线(渲染稳定/有意变更后)",
    )
    parser.add_argument("--json", dest="json_out", help="把检查 payload 写成 JSON")
    args = parser.parse_args(argv[1:])
    ref_dir = Path(args.ref_dir)

    if args.make_references:
        n = make_references(ref_dir, args.dpi)
        print(f"\n生成 {n} 份参考于 {ref_dir.relative_to(REPO_ROOT)}(dpi {args.dpi}, max_dim {REF_MAX_DIM})")
        return 0

    payload = check_references(ref_dir, args.dpi, args.min_ssim)
    if args.json_out:
        Path(args.json_out).write_text(json.dumps(payload, indent=2), encoding="utf-8")

    width = max((len(d["name"]) for d in payload["docs"]), default=8) + 4
    print(f"{'fixture__page':<{width}}{'SSIM':>8}  status")
    for d in payload["docs"]:
        page = f"__p{d['page']}"
        score = "  n/a " if d["ssim"] is None else f"{d['ssim']:.4f}"
        status = "ok" if d["passed"] else f"FAIL — {d['error']}"
        print(f"{d['name'] + page:<{width}}{score:>8}  {status}")
    n_pass = sum(1 for d in payload["docs"] if d["passed"])
    print(f"\n自渲染 SSIM 门: {n_pass}/{payload['n']} 页 ≥ {args.min_ssim} — {'PASS' if payload['passed'] else 'FAIL'}")
    return 0 if payload["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
