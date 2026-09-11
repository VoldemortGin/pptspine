# pdfspine v0.8.0 migration — 2026-09-10

`pdf-typeset` and test-only `pdf-fonts` now share the official git dependency
`https://github.com/VoldemortGin/pdfspine`, fixed at v0.8.0 commit
`f1f6ab4208876b0ba867edd76cc4e5da7ad8add2` instead of
`5f1640cb2640132ca3bb49d7e777bdeafe751668`. All six PDF crates in Cargo.lock
resolve to that source/version. No path dependency or sibling extension is used.
Other dependency identities, OCR rev `732975f0233cd6500edfbbb82bc06c2332369871`,
features and pptspine's `0.0.1` source version remain unchanged.

The `test` extra now declares `pdfspine>=0.8,<0.9` alongside pytest: the PDF
export tests require that reader, closing G11 without silently skipping them.
The built wheel's metadata contains this requirement. README's source build
uses the installed models package and maturin's uv installer.

No Rust API adaptation was required. The first rustfmt gate exposed a pre-existing
line-wrap mismatch in `ppt-render/src/lib.rs`; only that formatting was changed.
The suspected SSIM test path issue was disproved by execution: `parents[2]`
already points to the repository root. The original test passed unchanged.

## Checks performed

Host: macOS 26.5.1 arm64, Rust 1.96.0, Python 3.12.11. A separate target and
Python environment were used. Dependencies came from official git/PyPI sources
or their cache; dependency preparation may use the network.

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | passed after the single formatting correction |
| `cargo clippy --workspace --all-targets --all-features --offline -- -D warnings` | passed |
| `cargo test --workspace --exclude py-bindings --all-features --locked` | 102 passed |
| `cargo check -p py-bindings --all-features --locked` | passed |
| `maturin build --release --locked --strip` | wheel built and installed |
| `PPTSPINE_SSIM_GATE=1 python -m pytest python/tests -q -ra` | 59 passed, no skips |
| `python scripts/ocr_smoke.py python/tests/fixtures/ocr_sample.png` | all three reference lines recovered |
| `uv pip check` | passed |
| Committed self-reference SSIM, unchanged 0.97 threshold | 18/18 pages passed; minimum 0.9999 |

Rust coverage comprises core 18, OCR bridge 1, parsing/resolution 45 and render
38 tests. Python checks exercise the actual wheel's export/read-back geometry,
token-F1/order and nonblank raster gates, OCR and the enabled SSIM gate.
The 17 synthetic decks produce 18 reference pages. Three page scores round to
0.9999 (`minimal_4x3_text_table`, `widescreen_16x9_2slides` p0,
`e2e_inheritance_chain`); the other 15 round to 1.0000. All dimensions match,
and none regressed to blank. Reference files and thresholds were not changed.

The installed local candidate is
`pptspine-0.0.1-cp311-abi3-macosx_11_0_arm64.whl`, SHA-256
`79dc1f7d03d7de869796eb21f00bab579343867bf6d45bc72b2f2686cc1bb9a1`.
All five installed package payload files match the wheel byte-for-byte; package
and extension imports resolve to the external environment's site-packages.
This was not an editable install or a new published pptspine release.

Reader: PyPI pdfspine 0.8.0. Models: ocrspine-models 0.0.3. Build/test tools:
maturin 1.15.0 and pytest 9.1.1. Actual OCR weights come from the installed
`ocrspine_models` package; the smoke script's old “wheel-bundled models” message
does not describe the current packaging arrangement. OCR uses the repository's
own committed sample and recovered both Latin lines and the Chinese line.

LibreOffice 26.8.0.3 was additionally run on all 17 decks at 96 dpi as the
existing local advisory oracle. All page counts match, scores range
0.9356–1.0000, and no deck falls below the advisory 0.80 band. This is a
candidate-only observation, not a measured improvement over the previous
engine. Its read-only SSIM helper came from the pinned pdfspine release commit.

## Reproduction and coverage limits

From this repository, with Rust 1.96.0 and Python 3.12 available:

```bash
uv venv --python 3.12 /tmp/pptspine-validation-env
uv pip install --python /tmp/pptspine-validation-env/bin/python \
  maturin==1.15.0 pytest==9.1.1 pdfspine==0.8.0 ocrspine-models==0.0.3
export CARGO_TARGET_DIR=/tmp/pptspine-validation-target
export PYO3_PYTHON=/tmp/pptspine-validation-env/bin/python
/tmp/pptspine-validation-env/bin/maturin build --release --locked --strip \
  --out /tmp/pptspine-validation-wheels
uv pip install --python /tmp/pptspine-validation-env/bin/python \
  /tmp/pptspine-validation-wheels/*.whl
PPTSPINE_SSIM_GATE=1 /tmp/pptspine-validation-env/bin/python \
  -m pytest python/tests -q -ra
/tmp/pptspine-validation-env/bin/python scripts/ssim_baseline.py --min-ssim 0.97
/tmp/pptspine-validation-env/bin/python scripts/ocr_smoke.py \
  python/tests/fixtures/ocr_sample.png
```

SSIM is font-environment-sensitive; this local pass does not replace the
pinned Linux CI runner or the complete OS/Python wheel matrix. No upload,
cross-architecture build or arbitrary PowerPoint-layout equivalence is claimed.
README's separate editable-development command was checked for `--uv` and
`--extras` support, but this validation used the installed built wheel.

Evidence resides outside git under `/Volumes/ExternalSSD/tmp/`:
`pptspine-v080-{rust-gate-final,wheel-build,python-tests,ocr-smoke,ssim,lo}.log`,
`pptspine-v080-ssim.json`, `pptspine-v080-dependency-sources.json`,
`pptspine-v080-wheel-evidence.json` and `pptspine-v080-lo/`.
The initial formatting failure is retained in `pptspine-v080-rust-gate.log`.
The unchanged SSIM entry's successful probe is in
`pptspine-v080-ssim-entry-check.log`.
Shared family documents and sibling repositories were not modified.
