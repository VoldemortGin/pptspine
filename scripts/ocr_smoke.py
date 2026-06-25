#!/usr/bin/env python3
"""ocr_smoke — release gate that proves the BUILT wheel does real OCR.

Run AFTER installing the freshly built ``pptspine`` wheel into a clean
environment (so we exercise the wheel-bundled PP-OCRv5 ONNX models at
``site-packages/pptspine/_models``, exactly what an end user gets from
``pip install pptspine`` — no source tree, no extra, no network)::

    python scripts/ocr_smoke.py python/tests/fixtures/ocr_sample.png

It feeds the sample raster bytes straight to ``pptspine.ocr_image`` (the
top-level wrapper points the engine at the bundled models) and asserts the three
mixed CJK+Latin reference lines come back. Exit 0 on success, non-zero (with a
clear message) on any failure — so the CI ``wheels`` job fails and ``publish``
(which ``needs`` it) never runs. Pure stdlib; identical on Linux/macOS/Windows.
"""

from __future__ import annotations

import sys
from pathlib import Path

# Force UTF-8 stdout/stderr: this gate prints the recognized text (mixed
# CJK+Latin), but Windows Python defaults stdout to the legacy ANSI codepage
# (cp1252), which cannot encode '纯' & co. and would raise UnicodeEncodeError
# *after* OCR already succeeded. reconfigure() exists since 3.7; guarded so the
# script stays robust everywhere.
for _stream in (sys.stdout, sys.stderr):
    _reconfigure = getattr(_stream, "reconfigure", None)
    if _reconfigure is not None:
        _reconfigure(encoding="utf-8")

import pptspine

# The three lines printed in the OCR sample raster (must match
# python/tests/test_ocr.py — the canonical e2e fixture).
_LINE_LATIN_1 = "pdfspine OCR test 2026"
_LINE_CJK = "纯Rust实现的PDF文字识别"
_LINE_LATIN_2 = "PaddleOCR via tract"


def _contains_line(text: str, line: str) -> bool:
    """Whitespace-insensitive containment (CJK has no spaces, Latin may differ)."""
    norm = "".join(text.split())
    return "".join(line.split()) in norm


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(f"usage: {argv[0]} <ocr_sample.png>", file=sys.stderr)
        return 2
    sample = Path(argv[1])
    if not sample.exists():
        print(f"OCR sample raster missing: {sample}", file=sys.stderr)
        return 2

    print(f"pptspine {pptspine.__version__} — OCR release smoke on {sample}")

    items = pptspine.ocr_image(sample.read_bytes())
    if not items:
        print("FAIL: OCR returned no items at all", file=sys.stderr)
        return 1

    # Join every recognized line into one whitespace-insensitive blob.
    text = "\n".join(str(it["text"]) for it in items)
    print(f"recognized text:\n{text!r}")

    missing = []
    if not _contains_line(text, _LINE_LATIN_1):
        missing.append(_LINE_LATIN_1)
    if _LINE_CJK not in "".join(text.split()):
        missing.append(_LINE_CJK)
    if not _contains_line(text, _LINE_LATIN_2):
        missing.append(_LINE_LATIN_2)

    if missing:
        print(f"FAIL: OCR did not recover lines: {missing!r}", file=sys.stderr)
        return 1

    print("OK: PP-OCRv5 recovered all three lines from the wheel-bundled models.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
