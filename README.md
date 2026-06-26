# pptspine

[![PyPI](https://img.shields.io/pypi/v/pptspine.svg)](https://pypi.org/project/pptspine/)

A pure-Rust PowerPoint (`.pptx`) parser with Python bindings (PyO3 / maturin,
abi3-py311). A `.pptx` file is OOXML — a zip archive of XML parts — and pptspine
walks that XML directly to produce a structured, information-preserving model:
slides, text frames (paragraphs + styled runs), tables (cells, merges, fills),
pictures, and autoshapes. Embedded images can additionally be OCR'd locally,
offline, and deterministically via the sibling [`ocrspine`](../ocrspine) crate
(PP-OCRv5 through `tract-onnx` — no cloud, no network).

## Capabilities

| Area | Status |
| --- | --- |
| Slides + slide size | parsed |
| Text frames: paragraphs, runs, text | parsed |
| Run styling: font, size, bold, italic, solid-fill color | parsed |
| Paragraph level + alignment | parsed |
| Tables: rows, cells, cell text | parsed |
| Table merges: `gridSpan` / `rowSpan` / `hMerge` / `vMerge` | parsed |
| Cell solid-fill color | parsed |
| Pictures: `r:embed` rel → media name + raw bytes | parsed |
| Autoshapes: geometry name, fill, stroke, optional text | parsed (best-effort) |
| Groups (`p:grpSp`): recursive | parsed |
| Image OCR (embedded pictures → words + boxes) | working (`ocr_image`) |
| Image-table geometry reconstruction from OCR boxes | **deferred** (stub) |

Parsing is tolerant: unknown elements are skipped, missing attributes become
`None`, and malformed input yields a typed `PptError` rather than a panic.

## Install

```bash
pip install pptspine
```

pptspine is **on PyPI**. OCR works out of the box: the PP-OCRv5 weights ship in
the shared [`ocrspine-models`](https://pypi.org/project/ocrspine-models/) data
package — a runtime dependency `pip` pulls in automatically — so the wheel itself
ships no models. To build from source instead, see below.

## Build (from the package root)

```bash
uv venv .venv
VIRTUAL_ENV="$(pwd)/.venv" uv pip install maturin pytest
# Structural parsing needs no models. The OCR path resolves models from
# ../ocrspine/models by default (or set OCRSPINE_MODELS).
OCRSPINE_MODELS="$(cd ../ocrspine && pwd)/models" \
  VIRTUAL_ENV="$(pwd)/.venv" .venv/bin/maturin develop --release
```

## Use from Python

```python
import pptspine

pres = pptspine.open("deck.pptx")
print(pres.slide_count, pres.slide_size)   # e.g. 2 (9144000, 6858000)  # EMU

for slide in pres.slides():
    for shape in slide.shapes():           # list[dict], introspectable
        if shape["kind"] == "text":
            for para in shape["paragraphs"]:
                for run in para["runs"]:
                    print(run["text"], run["bold"], run["color"])
        elif shape["kind"] == "table":
            for row in shape["rows"]:
                print([cell["text"] for cell in row])
        elif shape["kind"] == "picture":
            print("image:", shape["media"])

# Run OCR on raw image bytes (PNG/JPEG), offline:
items = pptspine.ocr_image(open("scan.png", "rb").read())
print(" ".join(i["text"] for i in items))
```

## Rust workspace

```
crates/
  ppt-core    domain model + geometry (EMU) + typed PptError. No IO/zip/XML.
  ppt-parse   OOXML reader: zip extract + quick-xml walk -> Presentation.
  ppt-ocr     image-OCR bridge over ocrspine (PaddleOcr).
  py-bindings PyO3 _core extension (the FFI chokepoint).
```

## Deferred / follow-up

- Image-table geometry reconstruction from OCR boxes
  (`ppt_ocr::reconstruct_table_from_image`, currently a typed `Unsupported`
  stub).
- Richer color models (theme/scheme colors, gradients), hyperlinks, charts,
  SmartArt, notes/comments.
