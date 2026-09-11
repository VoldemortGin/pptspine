# pptspine

[![PyPI](https://img.shields.io/pypi/v/pptspine.svg)](https://pypi.org/project/pptspine/)

A pure-Rust PowerPoint (`.pptx`) parser with Python bindings (PyO3 / maturin,
abi3-py311). A `.pptx` file is OOXML — a zip archive of XML parts — and pptspine
walks that XML directly to produce a structured, information-preserving model:
slides, text frames (paragraphs + styled runs), tables (cells, merges, fills),
pictures, and autoshapes. A parsed deck can also be **exported to PDF**
(`to_pdf()` / `save_pdf()`, one page per slide) through the shared pure-Rust
`pdf-typeset` engine from the sibling
[`pdfspine`](https://github.com/VoldemortGin/pdfspine) — no LibreOffice, no
cloud converter. Embedded images can additionally be OCR'd locally,
offline, and deterministically via the sibling [`ocrspine`](../ocrspine) crate
(PP-OCRv5 through `tract-onnx` — no cloud, no network).

## Spine 家族 / Spine family

本仓库是 Spine 家族的成员之一（角色：L2 文档引擎）。家族全部成员、分层、依赖方向、依赖形式与当前差距见 [`docs/spine-family.md`](docs/spine-family.md)；该文件在每个家族仓库中的副本内容相同，真源在家族根目录 `~/startup/spine/docs/spine-family.md`，用根目录 `make family-doc-sync` 同步。

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
| Pictures: `r:embed` rel → media name; raw bytes via `Presentation.image_bytes()` | parsed |
| Autoshapes: geometry name, fill, stroke, optional text | parsed (best-effort) |
| Groups (`p:grpSp`): recursive | parsed |
| Speaker notes (`notesSlide` → `Slide.notes`) | parsed |
| Structured export: `to_text()` / `to_markdown()` (GFM + HTML tables for merges) | working |
| PDF export: `to_pdf()` / `save_pdf()` — one page per slide; placeholder/theme inheritance, shape transforms (rot/flip/adj/dash/`srcRect`), group affine, tables, slide backgrounds, body-anchor/autofit | working |
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
uv venv --python 3.12 .venv
uv pip install --python .venv/bin/python maturin
VIRTUAL_ENV="$(pwd)/.venv" .venv/bin/maturin develop --release --locked --uv --extras test
```

The `test` extra installs pytest and the pdfspine PDF read-back engine. Cargo
fetches `pdf-typeset` and test-only `pdf-fonts` from the same official pdfspine
v0.8.0 commit, `f1f6ab4208876b0ba867edd76cc4e5da7ad8add2`; no sibling checkout
is required. Installation may access the network; export and OCR run locally,
with model weights supplied by the installed `ocrspine-models` package.
See [migration validation](docs/pdfspine-v080-validation.md) for the actual
wheel, export/SSIM results and coverage limits.

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

# Structured export + speaker notes:
print(pres.to_text())          # slides joined by "--- slide N ---"
print(pres.to_markdown())      # one section per slide; GFM / HTML tables
print(pres.slides()[0].text)   # all text on a slide (convenience)
print(pres.slides()[0].notes)  # speaker notes, or None

# Run OCR on raw image bytes (PNG/JPEG), offline:
items = pptspine.ocr_image(open("scan.png", "rb").read())
print(" ".join(i["text"] for i in items))

# End-to-end: pull an embedded image's bytes and OCR them, offline:
for shape in pres.slides()[0].shapes():
    if shape["kind"] == "picture" and shape["media"]:
        data = pres.image_bytes(shape["media"])   # bytes | None
        if data:
            print([i["text"] for i in pptspine.ocr_image(data)])
```

## Export to PDF

```python
pres = pptspine.open("deck.pptx")
pres.save_pdf("deck.pdf")          # one PDF page per slide
pdf_bytes = pres.to_pdf()          # or in-memory bytes

# Optional: map a requested font family to a local font file (or to another
# installed family), layered on top of the built-in substitution table:
pres.save_pdf("deck.pdf", font_map={"Aptos": "/path/to/Aptos.ttf"})
```

Rendering is deterministic and fully offline. Missing fonts degrade gracefully:
an available face is substituted and a Python `UserWarning` is emitted **once
per warning kind** — the export never fails on a missing font.

## Rust workspace

```
crates/
  ppt-core    domain model + geometry (EMU) + typed PptError. No IO/zip/XML.
  ppt-parse   OOXML reader: zip extract + quick-xml walk -> Presentation.
  ppt-ocr     image-OCR bridge over ocrspine (PaddleOcr).
  ppt-render  slide -> PDF renderer over the shared pdf-typeset engine (from pdfspine).
  py-bindings PyO3 _core extension (the FFI chokepoint).
```

## Deferred / follow-up

- Image-table geometry reconstruction from OCR boxes
  (`ppt_ocr::reconstruct_table_from_image`, currently a typed `Unsupported`
  stub).
- Richer color models (gradients), hyperlinks, charts, SmartArt, comments.
