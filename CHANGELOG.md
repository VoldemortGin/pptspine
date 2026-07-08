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
