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
| Run styling: font, size, bold, italic, solid-fill color, character spacing `spc`, super/subscript `baseline`, caps `cap` (inherited through `lstStyle`/`txStyles`) | parsed |
| Paragraph level + alignment | parsed |
| Tables: rows, cells, cell text | parsed |
| Table merges: `gridSpan` / `rowSpan` / `hMerge` / `vMerge` | parsed |
| Cell solid-fill color | parsed |
| Pictures: `r:embed` rel → media name; raw bytes via `Presentation.image_bytes()`; alt text `cNvPr@descr` / `@title` / `@name` (`alt_text` / `title` / `name` keys) | parsed |
| Hyperlinks `a:hlinkClick` (run- and shape-level): external URL via rels, internal jumps (`ppaction://hlinksldjump` / `hlinkshowjump`) → target slide index (`hyperlink` key) | parsed; PDF: run-level external `http` / `https` / `mailto` links → URI link annotations |
| Placeholders: `p:ph` type/idx on every shape dict (`placeholder` key: `{"type", "idx"}` or `None`) | parsed |
| Hidden slides (`p:sld@show="0"` → `Slide.hidden`), sections (`p14:sectionLst` → `Presentation.sections()`), document properties (`docProps/core.xml` + `app.xml` → `Presentation.core_properties()`) | parsed |
| Autoshapes: geometry name, fill, stroke, optional text | parsed (best-effort) |
| Groups (`p:grpSp`): recursive | parsed |
| Chart data (`graphicFrame` > `c:chart` → `ppt/charts/chartN.xml`): kind (bar/line/pie/doughnut/area/scatter/bubble/radar/stock/surface, unknown → raw element name), title, categories, series name / values / `formatCode` from the cached `strCache`/`numCache`/literals (sparse `pt` padded to `ptCount`; external workbooks not read; missing cache → empty series + warning). `chart` key on placeholder dicts; `to_markdown()` emits `#### Chart: <title>` + table, `to_text()` `category: v, v` lines; dict also carries `bar_dir` / `grouping` / `three_d` / `combo` / `of_pie`; series dicts carry `color` / `point_colors` (`c:spPr` / `c:dPt` solid fills; line series: `a:ln` color) and `labels` (`c:dLbls` `showVal` / `showCatName` / `showPercent`) | parsed; PDF: vector bar/column (clustered/stacked/100%), line and pie charts (file series / per-point colors, else theme accent colors; value / category / percent data labels; axis, legend, title); other kinds / 3D / combo / `ofPie` → placeholder box + `chart-degraded` warning |
| Speaker notes (`notesSlide` → `Slide.notes`) | parsed |
| SmartArt (`graphicFrame` > `dgm:relIds`): the pre-rendered drawing part (`ppt/diagrams/drawingN.xml`, found via the data part's `dsp:dataModelExt@relId` or the slide's `diagramDrawing` rel) is parsed with the regular shape parser into a group placed by the frame's `xfrm`; without a usable drawing the data part's content-point text (`dgm:pt > dgm:t`, document order, `pres` / `parTrans` / `sibTrans` points skipped) goes to a `text` list on the placeholder dict; both missing → empty placeholder. Text flows into `to_text()` / `to_markdown()` | parsed; PDF: drawing shapes render like any group; data-only / missing → placeholder box + `smartart-degraded` warning |
| Slide comments (`ppt/comments/commentN.xml` legacy `p:cmLst` with `ppt/commentAuthors.xml`, and modern threaded `p188:cmLst` with replies and `ppt/authors.xml`, both located via the slide's rels): `Slide.comments()` → `list[dict]` with `author` / `initials` / `datetime` / `text` / `position` (legacy `p:pos`, raw units) / `replies`; missing attributes → `None`. Review metadata: **not** in `text` / `to_text()` / `to_markdown()` and not drawn in the PDF (same trade-off as docspine); authors and bodies never appear in warnings | parsed |
| OLE objects (`graphicFrame` > `a:graphicData[@uri=.../ole]` > `p:oleObj`, bare or wrapped in `mc:AlternateContent`): the `p:oleObj > p:pic` preview image enters the model as a picture at the frame's `xfrm` (an `AlternateContent` branch holding only a bare `p:embed` OLE frame yields to the branch that carries a preview); no preview → placeholder box | parsed; PDF: drawn as a normal picture; EMF / WMF previews follow the existing unsupported-image path (skipped + `ImageDropped` warning) |
| Structured export: `to_text()` / `to_markdown()` — visual reading order (XY-cut over flattened group geometry; `order="document"` falls back to z-order), title from `title`/`ctrTitle` placeholder, list markers `- ` / `1. ` from the resolved bullet chain, `![alt](media)` images, `[text](url)` external links, GFM + HTML tables for merges; hidden slides skipped unless `include_hidden=True` | working |
| PDF export: `to_pdf()` / `save_pdf()` — one page per visible slide (hidden slides skipped unless `include_hidden=True`, matching PowerPoint); placeholder/theme inheritance, shape transforms (rot/flip/adj/dash/`srcRect`), line ends (`headEnd`/`tailEnd`: triangle/stealth/diamond/oval/arrow), group affine, tables (incl. `tableStyles.xml` fills / borders / text color+bold, header / banded rows & columns), slide backgrounds, slideMaster/slideLayout graphics (non-placeholder logos, bars, rules; `showMasterSp`), slide-number fields (`slidenum` = position + `firstSlideNum` − 1, hidden slides keep their number; `datetime*` fields keep the cached text) in the footer / slide-number / date placeholders the slide instantiates, body-anchor/autofit, superscript/subscript with the document's own baseline offset, character spacing (expanded and condensed), `cap=all` (`cap=small` approximated as all caps + warning) | working |
| `custGeom` freeform shapes | degraded: bounding-box rect (connector: straight line) + `custom-geometry-approximated` warning |
| Image OCR (embedded pictures → words + boxes) | working (`ocr_image`) |
| Image-table geometry reconstruction from OCR boxes | **deferred** (stub) |

Parsing is tolerant: unknown elements are skipped, missing attributes become
`None`, and malformed input yields a typed `PptError` rather than a panic.

Untrusted input is bounded. The zip reader never trusts declared entry sizes
(no pre-allocation from header fields; reads are capped at the limit) and
rejects packages that exceed `ppt_parse::ZipLimits` with
`PptError::LimitExceeded` (`PptZipError` in Python, message names the limit).
Defaults: 10,000 entries, 256 MiB per entry, 1 GiB total decompressed, a
compression ratio of 10,000 (only checked for entries over 1 MiB) and 1024-byte
entry names, and 5,000 slides (counted after de-duplicating repeated `p:sldIdLst` references). Absolute, drive-letter (`C:`) or `..` entry paths are rejected, and group /
`mc:AlternateContent` nesting deeper than 64 levels is skipped instead of
recursing. Rust callers can pass custom limits via `parse_bytes_with_limits` /
`parse_path_with_limits`.

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
v0.11.2 commit, `78a64d6e252ab739fcbad66c0d7f5328a080d667`, and `ocrspine` from
commit `041958aa6f8d70d3957e8f9e27896cf0cbc42511`; no sibling checkout
is required. Installation may access the network; export and OCR run locally,
with model weights supplied by the installed `ocrspine-models` package.
See [migration validation](docs/pdfspine-v0112-validation.md) for the actual
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
            print("image:", shape["media"], shape["alt_text"])
        print(shape["placeholder"])        # {"type": "title", "idx": None} | None

# Structured export + speaker notes (visual reading order, hidden slides skipped):
print(pres.to_text())          # slides joined by "--- slide N ---"
print(pres.to_markdown())      # "### <title placeholder>", "- " / "1. " lists, ![alt](media), [text](url)
print(pres.to_text(order="document", include_hidden=True))  # z-order, keep hidden slides
print(pres.slides()[0].text)   # all text on a slide (convenience, visual order)
print(pres.slides()[0].notes)  # speaker notes, or None
print(pres.slides()[0].hidden) # p:sld@show="0"
print(pres.sections())         # [("Intro", [0]), ("Body", [1, 2])] — [] without sections
print(pres.core_properties()["title"])  # docProps core/app fields; missing → None

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
pres.save_pdf("all.pdf", include_hidden=True)  # hidden slides are skipped by default

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

## Fuzzing

`fuzz/` is a standalone cargo-fuzz package (excluded from the workspace; needs nightly and
`cargo install cargo-fuzz`). Only panics / aborts / OOM count as failures — any `Err` is fine.
A daily CI job (`.github/workflows/fuzz.yml`) runs every target; it is not part of `ci.yml`.
Always invoke `cargo +nightly fuzz ...` explicitly: `rust-toolchain.toml` pins stable and would
otherwise override the default toolchain.

```bash
cargo run --manifest-path fuzz/Cargo.toml --bin make_seeds   # synthesize seeds into fuzz/corpus/ (git-ignored)
cargo +nightly fuzz run parse_slide_xml -- -max_total_time=120 -rss_limit_mb=2048
cargo +nightly fuzz run parse_pptx      -- -max_total_time=120 -rss_limit_mb=2048
cargo +nightly fuzz run render_pdf      -- -max_total_time=120 -rss_limit_mb=2048
```

Reproduce a crash with `cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<crash-file>` and
shrink it with `cargo +nightly fuzz tmin <target> <crash-file>`. To add a target: create
`fuzz/fuzz_targets/<name>.rs`, register a `[[bin]]` in `fuzz/Cargo.toml`, add it to the matrix in
`fuzz.yml`, and add seeds in `fuzz/seed.rs`. Fixed crashes get a regression test built in code
(in `crates/ppt-parse/tests/fuzz_regressions.rs`, created with the first fix), never a committed binary.

## Deferred / follow-up

- Image-table geometry reconstruction from OCR boxes
  (`ppt_ocr::reconstruct_table_from_image`, currently a typed `Unsupported`
  stub).
- Richer color models (gradients), hyperlinks, charts.
