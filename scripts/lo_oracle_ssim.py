#!/usr/bin/env python3
"""LibreOffice oracle SSIM advisory — **local-only, never CI**(PRD-PDF-EXPORT B-11 第 (3) 层)。

对导出 fixture 矩阵(或命令行给定的任意 .pptx):

1. LibreOffice ``soffice --headless --convert-to pdf`` 生成 oracle PDF;
2. ``pptspine`` ``to_pdf()`` 生成我们的 PDF;
3. 双方均用 ``pdfspine`` ``get_pixmap`` 栅格化,逐页算 SSIM(实现复用 pdfspine
   conformance ``render_diff.py`` 的纯 Python 窗口化 SSIM,**只读引用,不改**);
4. 打印逐 fixture 的 SSIM 表,按 advisory band 0.80–0.90 归类。

SSIM 只是 advisory:两侧字体替换、LO 自身的排版差异都会压低分数,**绝不做 CI 门**。
明显异常(< 0.80)才值得人工看渲染图找原因;已知 v1 降级(未知预设 → 包围盒、
纵排 → 水平)本来就会低,属预期。

用法(仓库根)::

    .venv/bin/python scripts/lo_oracle_ssim.py                 # 跑内置 fixture 矩阵
    .venv/bin/python scripts/lo_oracle_ssim.py deck1.pptx ...  # 跑任意 pptx
    .venv/bin/python scripts/lo_oracle_ssim.py --save-png      # 落 PNG 供目检

输出与中间产物落在 ``--out-dir``(缺省 ``.lo-oracle/``,已 gitignore)。
"""

from __future__ import annotations

import argparse
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import warnings
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
SOFFICE_DEFAULT = "/Applications/LibreOffice.app/Contents/MacOS/soffice"
RENDER_DIFF_DEFAULT = REPO_ROOT.parent / "pdfspine" / "conformance" / "gt" / "render_diff.py"
ADVISORY_LO, ADVISORY_HI = 0.80, 0.90


def _load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot import {name} from {path}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _fixture_matrix(conftest) -> dict[str, bytes]:
    """把导出 fixture 矩阵物化成 ``名字 -> pptx 字节``(取每对 deck 的主变体)。"""

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


def _soffice_convert(soffice: str, pptx: Path, out_dir: Path, profile: Path) -> Path:
    """``soffice --headless --convert-to pdf``;独立 UserInstallation 免与桌面实例互锁。"""
    cmd = [
        soffice,
        f"-env:UserInstallation=file://{profile}",
        "--headless",
        "--norestore",
        "--convert-to",
        "pdf",
        "--outdir",
        str(out_dir),
        str(pptx),
    ]
    res = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    pdf = out_dir / (pptx.stem + ".pdf")
    if res.returncode != 0 or not pdf.exists():
        raise RuntimeError(f"soffice failed on {pptx.name}: {res.stderr.strip() or res.stdout.strip()}")
    return pdf


def _gray_pages(pdfspine, rd, pdf_bytes: bytes, dpi: float) -> list[tuple[int, int, list[float]]]:
    doc = pdfspine.open(stream=pdf_bytes, filetype="pdf")
    pages = []
    zoom = dpi / 72.0
    for page in doc:
        pm = page.get_pixmap(matrix=pdfspine.Matrix(zoom, zoom))
        samples = bytes(pm.samples)
        n = pm.n
        if n == 4:
            samples, n = rd._drop_alpha(samples, pm.width, pm.height), 3
        pages.append(rd._to_gray_downsampled(pm.width, pm.height, n, samples, max_dim=512))
    return pages


def _ssim_pdfs(pdfspine, rd, ours: bytes, oracle: bytes, dpi: float) -> tuple[float, int, int, list[float]]:
    """返回 ``(mean_ssim, our_pages, oracle_pages, per_page)``(只比对共同页数)。"""
    a_pages = _gray_pages(pdfspine, rd, ours, dpi)
    b_pages = _gray_pages(pdfspine, rd, oracle, dpi)
    per_page: list[float] = []
    for (aw, ah, apx), (bw, bh, bpx) in zip(a_pages, b_pages):
        tw, th = min(aw, bw), min(ah, bh)
        afit = rd._resize_gray(aw, ah, apx, tw, th)
        bfit = rd._resize_gray(bw, bh, bpx, tw, th)
        per_page.append(rd.ssim(afit, bfit, tw, th))
    mean = sum(per_page) / len(per_page) if per_page else 0.0
    return mean, len(a_pages), len(b_pages), per_page


def _save_pngs(pdfspine, pdf_bytes: bytes, prefix: Path, dpi: float) -> None:
    doc = pdfspine.open(stream=pdf_bytes, filetype="pdf")
    zoom = dpi / 72.0
    for i, page in enumerate(doc):
        page.get_pixmap(matrix=pdfspine.Matrix(zoom, zoom)).save(f"{prefix}-p{i + 1}.png")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("pptx", nargs="*", help="额外/替代的 .pptx 路径;缺省跑内置 fixture 矩阵")
    parser.add_argument("--out-dir", default=str(REPO_ROOT / ".lo-oracle"))
    parser.add_argument("--soffice", default=os.environ.get("SOFFICE", SOFFICE_DEFAULT))
    parser.add_argument("--render-diff", default=os.environ.get("RENDER_DIFF", str(RENDER_DIFF_DEFAULT)))
    parser.add_argument("--dpi", type=float, default=96.0)
    parser.add_argument("--save-png", action="store_true", help="双方逐页 PNG 落盘供目检")
    args = parser.parse_args(argv[1:])

    if not Path(args.soffice).exists():
        raise SystemExit(f"soffice not found: {args.soffice} (local-only advisory; 装 LibreOffice 后再跑)")

    import pdfspine

    import pptspine

    rd = _load_module("render_diff", Path(args.render_diff))

    if args.pptx:
        decks = {Path(p).stem: Path(p).read_bytes() for p in args.pptx}
    else:
        conftest = _load_module("conftest", REPO_ROOT / "python" / "tests" / "conftest.py")
        decks = _fixture_matrix(conftest)

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="lo-profile-"))

    print(f"soffice: {args.soffice}")
    print(f"decks: {len(decks)} · dpi {args.dpi} · advisory band {ADVISORY_LO:.2f}-{ADVISORY_HI:.2f}\n")
    rows: list[tuple[str, float, str]] = []
    try:
        for name, pptx_bytes in decks.items():
            pptx_path = out_dir / f"{name}.pptx"
            pptx_path.write_bytes(pptx_bytes)
            try:
                oracle_pdf = _soffice_convert(args.soffice, pptx_path, out_dir, profile)
            except (RuntimeError, subprocess.TimeoutExpired) as exc:
                rows.append((name, float("nan"), f"oracle FAILED: {exc}"))
                continue
            with warnings.catch_warnings():
                warnings.simplefilter("ignore")
                ours = pptspine.open_bytes(pptx_bytes).to_pdf()
            (out_dir / f"{name}.ours.pdf").write_bytes(ours)
            oracle_bytes = oracle_pdf.read_bytes()
            mean, np_a, np_b, per_page = _ssim_pdfs(pdfspine, rd, ours, oracle_bytes, args.dpi)
            note = "" if np_a == np_b else f"page-count mismatch ours={np_a} oracle={np_b}"
            if args.save_png:
                _save_pngs(pdfspine, ours, out_dir / f"{name}.ours", args.dpi)
                _save_pngs(pdfspine, oracle_bytes, out_dir / f"{name}.oracle", args.dpi)
            rows.append((name, mean, note))
    finally:
        shutil.rmtree(profile, ignore_errors=True)

    width = max(len(n) for n, _, _ in rows) + 2
    print(f"{'fixture':<{width}}{'SSIM':>7}  band")
    low = 0
    for name, score, note in rows:
        if score != score:  # NaN
            band = "ERROR"
        elif score < ADVISORY_LO:
            band, low = "BELOW band — inspect", low + 1
        elif score <= ADVISORY_HI:
            band = "in advisory band"
        else:
            band = "above band"
        extra = f"  ({note})" if note else ""
        print(f"{name:<{width}}{score:7.4f}  {band}{extra}")
    print(f"\n{low} deck(s) below {ADVISORY_LO:.2f} — advisory only, never a CI gate.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
