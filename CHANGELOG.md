# Changelog

All notable changes to **pptspine** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

pptspine is an Apache-2.0-licensed, pure-Rust PowerPoint (`.pptx`) reader with
PyO3 Python bindings and a fidelity-preserving **PDF export** built on the
shared `pdf-typeset` engine from [pdfspine](https://pypi.org/project/pdfspine/).
It is **alpha / pre-1.0**: the core is feature-complete, but the public API may
still change.

## [Unreleased]

### Added

- Semantic extraction: visual reading order for `to_text`/`to_markdown`/`Slide.text` (XY-cut over flattened group geometry; `order="document"` restores z-order); Markdown titles from `title`/`ctrTitle` placeholders, `- `/`1. ` list markers from the resolved bullet chain, `![alt](media)` images (`cNvPr@descr`, falls back to `@name`), `[text](url)` external hyperlinks; `shapes()` dicts gain `placeholder`, `hyperlink`, `alt_text`/`title`/`name`; run dicts gain `hyperlink`; `Slide.hidden`, `Presentation.sections()`, `Presentation.core_properties()`.
- PDF export draws slideMaster / slideLayout non-placeholder shapes (logos, decorative bars, footer rules, fixed footer text) beneath slide content, honoring `showMasterSp` on slides (hides master + layout graphics) and layouts (hides master graphics). Master/layout pictures and picture backgrounds now resolve through their own part rels (previously dropped silently). `Slide.shapes()`, `to_text()` and `to_markdown()` remain slide-only.
- Run-level `a:rPr@spc` (signed character spacing), `@baseline` (superscript/subscript using the document's own offset; glyph ×0.65) and `@cap` are parsed, inherited through `txStyles`/`lstStyle`, exposed in the run dict (`char_spacing_pt` / `baseline` / `cap`) and rendered via pdf-typeset `CharacterSpacing` / `ResolvedScriptPlacement`. Over-condensed paragraphs fall back with a `SignedSpacingFallback` warning; `cap=small` is approximated as all caps with a one-time `small-caps` warning. Default decks render byte-identical PDFs.
- PDF export draws line ends `a:headEnd` / `a:tailEnd` (triangle / stealth / diamond / oval / arrow; sm/med/lg sized per LibreOffice ratios) on open outlines, shortening the stroke to the arrow base; inherited via `lnRef` → theme `lnStyleLst`. Unsupported kinds degrade with a `line-end-degraded` warning.
- Chart data extraction: `graphicFrame` charts now read `ppt/charts/chartN.xml` cached data (`strCache`/`numCache`/literals; no external workbook) — kind, title, categories, series name/values/`formatCode`; sparse `pt` padded to `ptCount`; missing caches yield empty series + `warnings`. Exposed as the `chart` key on placeholder shape dicts, `#### Chart: <title>` + table in `to_markdown()`, and `category: v, v` lines in `to_text()`. PDF rendering unchanged (placeholder box).
- Fuzzing: standalone cargo-fuzz package (`fuzz/`) with `parse_pptx` / `parse_slide_xml` / `render_pdf` targets, in-code seed generator, and a daily `fuzz.yml` workflow (nightly toolchain, not part of `ci.yml`). No crashes found in initial local runs.

### Changed

- Hidden slides (`show="0"`) are skipped by `to_text`/`to_markdown`/`to_pdf`/`save_pdf` by default (`include_hidden=True` to keep them), matching PowerPoint's PDF export. Text export order defaults to visual reading order. Markdown list indent is now `level × 2` spaces.
- `a:custGeom` is no longer silently drawn as a rectangle — it is approximated by its bounding box (connectors as a straight line) with a `custom-geometry-approximated` warning; custGeom colored only via `p:style` is now drawn too.
- Python export warnings of the `Custom` kind now surface once per kind (previously collapsed into one).
- Align PDF export dependencies with pdfspine v0.8.0 (2026-09-10, `91e0255`;
  previously unrecorded): `pdf-typeset` / `pdf-fonts` git deps and the
  validation notes in `docs/pdfspine-v080-validation.md`.
- Align git deps to ocrspine `041958a` (fixes a reading-order sort panic) and
  pdfspine v0.11.2 (`78a64d6`, pdf-typeset / pdf-fonts); the test extra now
  accepts `pdfspine>=0.8,<0.12`. No behavior change; the 18 SSIM
  self-reference pages are unchanged.

### Fixed

- CI wheels job "Wheel OCR smoke" had failed since 2026-07 (the `--no-index`
  install could not resolve the hard dependency `ocrspine-models`); it now
  installs the models package first, then the locally built wheel with
  `--no-index`. The pytest job installs `.[test]` and runs with `-ra`.
- `ppt-ocr` isolates panics from the ocrspine engine, image decode and engine
  construction into `PptError::Ocr` (Python `PptOcrError`) instead of letting
  them escape as `PanicException`; words with NaN/Inf bbox or confidence are
  dropped.

### Security

- The zip reader no longer pre-allocates from the entry's declared size, and
  reads are capped. New `ZipLimits` (defaults: 10 000 entries, 256 MiB per
  entry, 1 GiB total, compression ratio 10 000 for entries > 1 MiB, 1024-byte
  names) -> `PptError::LimitExceeded` (Python `PptZipError`). Absolute, drive-letter and `..`
  entry paths are rejected. Group-shape / `mc:AlternateContent` nesting deeper
  than 64 is skipped to avoid stack overflow. New `parse_bytes_with_limits` /
  `parse_path_with_limits`.

## [0.5.1] — 2026-07-30

### Changed

- **重新放宽 Python 支持到 3.12+。** `requires-python` 从 `>=3.14,<3.15` 放宽为
  `>=3.12`（无上界），classifiers 恢复 3.12 / 3.13 / 3.14；CI pytest 矩阵覆盖
  3.12–3.14，固定版 setup-python 与 SSIM 门改用 3.12。wheel 仍是 abi3-py311
  构建，二进制不变。

## [0.5.0] — 2026-07-29

### Changed

- **BREAKING: 仅支持 Python 3.14。** `requires-python` 收紧为 `>=3.14,<3.15`，
  classifiers 只保留 3.14；CI 测试矩阵与 release 工作流全部固定在 3.14。
  wheel 仍是 abi3-py311 构建，但元数据层面只允许安装到 Python 3.14。

## [0.4.0] — 2026-07-13

### Added

- **Recompute autofit for text boxes (B-6).** When `a:normAutofit` is on but no
  `fontScale` was stored, the exported text now shrinks to fit its box: driven by
  the engine's TS-10 `measure_text_box`, it steps `fontScale` down (95%→25%) and
  escalates `lnSpcReduction` (0→10→20%), taking the first combination whose
  measured content height fits — the scale is baked into the runs so the engine
  does not re-shrink. A **stored** `normAutofit@fontScale` keeps its
  applied-as-stored behavior (no regression).
- **Content-adaptive table row height (B-7).** Each row now grows to
  `max(declared/frame-derived height, measured content height)` via TS-10
  `measure_blocks`, so long cell text is no longer clipped; `rowSpan` cells
  distribute their content height evenly across the rows they span. Rows whose
  content fits render byte-identically to before (no unintended drift).
- Bumped the pinned `pdf-typeset` / `pdf-fonts` engine rev to pdfspine v0.3.1
  (`5f1640c`), which adds the public TS-10 measurement API both features consume.

- **Self-render SSIM regression gate (B-11 gate (4)).** Committed grayscale
  references (`python/tests/ssim_refs/*.ssimref`) captured from our own
  `to_pdf()` output on a pinned-font runner; CI re-renders the full synthetic
  fixture matrix, rasterises through pdfspine and asserts per-page
  SSIM ≥ 0.97 (plus size-match and no near-blank regression). Unlike the
  LibreOffice oracle (advisory, never-CI), this is a **CI-blocking** gate that
  turns any unintended render drift into a red build. Regenerate deliberately
  after an intended render change with
  `python scripts/ssim_baseline.py --make-references`. The pytest gate is
  font-runner-sensitive, so it enforces on ubuntu / py3.12 only (gated by
  `PPTSPINE_SSIM_GATE`); the rest of the matrix skips it. CI now installs
  `pdfspine>=0.3` for the read-back gates.

## [0.3.0] — 2026-07-08

### Added

- **Full table fidelity in PDF export (B-7).** `a:tcPr` per-side borders, cell
  margins and vertical anchoring parsed and rendered as stroked lines; merged
  cells (`gridSpan`/`rowSpan`) suppress internal dividers. A missing `a:tblGrid`
  now degrades to even-width columns (never dropping the table) with a single
  `table-grid` warning.
- **Slide-background inheritance (B-10).** `bg`/`bgPr` resolved along the
  slide → layout → master chain, including `bgRef` via the theme; gradient
  backgrounds degrade with one `GradientDegraded` warning.
- **Warning-surfacing audit (B-11).** Exactly one `warnings.warn` per unique
  degradation kind; `font_map` accepts filesystem font paths; vertical text
  degrades to horizontal with a single warning; `spcPct` line spacing converted.
- **LibreOffice oracle SSIM advisory** (`scripts/lo_oracle_ssim.py`) — a
  local-only, never-CI script that rasterises our export and a `soffice
  --headless` reference through pdfspine and reports a windowed SSIM per fixture
  (advisory band 0.80–0.90). The synthetic fixture matrix currently scores
  0.94–1.00 against LibreOffice.

## [0.2.0] — 2026-07-04

### Added

- **Fidelity-preserving PDF export** — `Presentation.to_pdf()` /
  `save_pdf()`, one PDF page per slide, drawn through the shared pure-Rust
  `pdf-typeset` engine (git-pinned pdfspine crates).
  - Placeholder inheritance chain + theme subsystem resolved into a
    `ResolvedPresentation` IR (B-8/B-9).
  - Shape transforms: rotation, flips, preset-geometry adjust values, dashed
    strokes, `srcRect` image crop (B-4); group affine remap so grouped and
    ungrouped twins render identically (B-5).
  - Text-box `bodyPr` anchoring, insets and stored-autofit scale (B-6).
  - `font_map` override and per-kind degradation warnings — export never fails
    on a missing font.
- Embedded-image byte round-trip, OCR engine caching, structured export
  (`to_text()` / `to_markdown()`), and speaker-notes extraction.

### Fixed

- Intel-mac wheels build via `macos-14` cross-compilation so releases cover the
  full platform matrix.
- `cargo test` excludes `py-bindings` to avoid the macOS abi3 link failure.

## [0.1.1] — 2026-06-30

### Fixed

- Corrected `NOTICE`: OCR models ship via the `ocrspine-models` package, not
  bundled into the wheel.

## [0.1.0] — 2026-06-26

### Added

- Initial release: pure-Rust `.pptx` reader with text, table, and image
  extraction; `to_text()` / `to_markdown()` structured export; optional OCR of
  embedded raster images via the shared `ocrspine` engine; PyO3 bindings with
  abi3 wheels for macOS, Linux, and Windows.
