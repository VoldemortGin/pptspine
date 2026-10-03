# pdfspine v0.11.2 / ocrspine 041958a dependency alignment — 2026-10-02

Behavior-preserving bump of pptspine's three git dependency revs, aligned with
the sibling docspine. No Rust or Python source changed; no new pdfspine
capability (character spacing, super/subscript, paragraph borders, ...) is wired
in. That migration is separate work.

## Rev changes

| Dependency | Before | After |
| --- | --- | --- |
| `ocrspine` | `732975f0233cd6500edfbbb82bc06c2332369871` | `041958aa6f8d70d3957e8f9e27896cf0cbc42511` |
| `pdf-typeset` | `f1f6ab4208876b0ba867edd76cc4e5da7ad8add2` (v0.8.0) | `78a64d6e252ab739fcbad66c0d7f5328a080d667` (v0.11.2) |
| `pdf-fonts` | `f1f6ab4208876b0ba867edd76cc4e5da7ad8add2` (v0.8.0) | `78a64d6e252ab739fcbad66c0d7f5328a080d667` (v0.11.2) |

The `test` extra's reader range changed from `pdfspine>=0.8,<0.9` to
`pdfspine>=0.8,<0.12`.

The v0.11.2 hash was checked with `git -C ../pdfspine rev-parse v0.11.2^{commit}`
and matches. `Cargo.lock` was updated with `cargo update -p ... --precise`; all
six pdfspine crates (`pdf-core`, `pdf-edit`, `pdf-fonts`, `pdf-image`,
`pdf-text`, `pdf-typeset`) resolve to `0.11.2` at the single commit above, and
`ocrspine` to the new rev. The only other lock change is a new `sha2`
dependency edge of a pdfspine crate (`sha2` was already locked). No other crate
versions moved.

## Validation

On macOS arm64, Rust 1.96, Python 3.12, installed `pdfspine==0.11.2`,
`ocrspine-models==0.0.3`, maturin 1.15.0, pytest 9.1.1:

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | passed |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | passed, no source change needed |
| `cargo test --workspace --exclude py-bindings --all-features --locked` | 102 passed (core 18, OCR bridge 1, parse 45, render 38), same as v0.8.0 |
| `maturin develop --release --locked` | built and installed (editable) |
| `PPTSPINE_SSIM_GATE=1 pytest python/tests -q -ra` | 59 passed, 0 skipped |
| `python scripts/ocr_smoke.py python/tests/fixtures/ocr_sample.png` | all three reference lines recovered |
| `python scripts/ssim_baseline.py --min-ssim 0.97` | 18/18 pages passed |

SSIM, per page (baselines not regenerated, threshold unchanged at 0.97):
minimal_4x3_text_table 0.9999, widescreen_16x9_2slides p0 0.9999 / p1 1.0000,
e2e_inheritance_chain 0.9999; the other 14 pages 1.0000. Identical to the
v0.8.0 record; no page dropped.

## OCR output

`test_ocr.py` uses the sample vendored in `python/tests/fixtures/`, so it ran
(not skipped). `ocr_smoke.py` output with the new rev:
`pdfspine OCR test 2026`, `纯Rust实现的PDF文字识别`, `PaddleOCR via tract`.
Reference lines are unchanged, so the ocrspine `e810a9c` mid-gray fill change
caused no drift and no reference line was edited.

## Limits

One host/Python combination only; not the Linux/macOS/Windows and Python
3.11-3.14 CI matrix, not a wheel install check, not a package upload. SSIM is
the self-reference gate against committed baselines (it detects change from the
previous output, not fidelity against PowerPoint/LibreOffice); the LibreOffice
advisory comparison was not rerun. The Python reader installed was
`pdfspine==0.11.2` (allowed range `>=0.8,<0.12`).
