# PRD — Faithful PDF Export (.pptx → PDF)

Status: **B-1..B-11 已基本实现落地(code-verified)** · Date: 2026-07-02 · Repo: `/Users/linhan/workspace/spine/pptspine`
Family position: **Phase B** of the spine PDF-export program — Phase A is the shared engine
`crates/pdf-typeset` in pdfspine (`pdfspine docs/PRD-NEXT.md` §10, tasks TS-1..TS-7); Phase C is docspine.
pptspine ships first because per-slide absolute positioning avoids pagination entirely.
Effort scale: **S** ≈ hours · **M** ≈ 1–2 days · **L** ≈ multi-day. Each task lists a concrete
**green condition**. All verdicts carry `file:line` evidence against the current working trees.

---

## 1. Goal & non-goals

**Goal (fidelity definition).** `Presentation.to_pdf()` produces one PDF page per slide, at the exact
slide size (EMU → pt, `EMU_PER_POINT = 12700`, `crates/ppt-core/src/geom.rs:7-57`), with every shape
drawn at its absolute resolved position: text in the correct font/size/style/color at the correct
coordinates, shapes with correct geometry/fill/stroke, pictures placed and cropped, tables with real
column geometry and borders, and the slideLayout/slideMaster/theme inheritance chain resolved so that
placeholder text looks like it does in PowerPoint. Output is extractable (ToUnicode always written) and
deterministic **per font environment** (same machine + fonts ⇒ same bytes).

**Explicit disclaimer.** The existing content-level path — `to_markdown()`
(`crates/ppt-core/src/export.rs:38-44`) piped into pdfspine `markdown_to_pdf` — does **NOT** count as
export fidelity. It discards all geometry. This PRD is about true layout-faithful rendering only.

**Locked family decisions (do not reopen):**
- All PDF drawing/typesetting lives in pdfspine `crates/pdf-typeset` (Phase A). pptspine adds exactly
  **one** new crate `crates/ppt-render` mapping the ppt-core IR onto pdf-typeset. ppt-core stays
  dependency-free; ppt-render depends on ppt-core + pdf-typeset; py-bindings grow a `to_pdf` path.
- Dependency form follows the family ocrspine precedent (`Cargo.toml:24-26`, verbatim):
  ```toml
  # --- sibling family crate: domain-neutral OCR (single source of truth for the
  # path; ppt-ocr uses `ocrspine.workspace = true` so no per-crate path math). ---
  ocrspine = { git = "https://github.com/VoldemortGin/ocrspine", rev = "732975f0233cd6500edfbbb82bc06c2332369871" }
  ```
  ⇒ `pdf-typeset = { git = "https://github.com/VoldemortGin/pdfspine", rev = "<pinned>" }`.
  (Note: `CLAUDE.md` still describes ocrspine as a path dep — `Cargo.toml` is ground truth.)
- Python API v1 is locked to §6. Fixture policy is locked to synthesized decks via
  `python/tests/conftest.py::_zip_pptx` (`conftest.py:160-166`) — no binary .pptx fixtures.
- Phase order A → **B (this PRD)** → C. Phase B engine prerequisites: **TS-2** (system fonts),
  **TS-3** (multi-face + subsetter), **TS-5** (text boxes), **TS-6** (preset geometry).

**v1 OUT of scope** (each with rationale + mandatory degradation behavior — degrade-never-panic):

| Item | Rationale | v1 degradation |
|---|---|---|
| Charts (`graphicFrame` > `c:chart`) | chart rendering is its own engine | bounding-box placeholder rect + warning. ⚠ today the shape vanishes **including its rect** (`slide.rs:494-500`); parse must at least capture the frame rect (B-3) |
| SmartArt (`dgm:relIds`) | same | same placeholder + warning |
| Audio/video/OLE | no static-PDF equivalent | video `p:pic` renders its poster-frame blip; else placeholder + warning |
| Animations/transitions | N/A in PDF | silently ignored (final build state rendered) |
| WordArt text effects | glyph outline/fill machinery | plain styled text + warning |
| Vertical text (`bodyPr@vert` ≠ `horz`) | vertical layout engine deferred | render horizontally + warning |
| `custGeom` | formula evaluator + path builder is L | bounding-box rect with the shape's fill/line + warning; text still rendered on top |
| Gradient fills (`gradFill`) | pdf-edit authoring has no `/Shading` (pdfspine `crates/pdf-edit`, grep-verified) | representative solid (first stop) + warning |
| Shadows / 3D / reflection (`effectLst`) | effects pipeline out | dropped + warning |
| `tableStyles.xml` full banding semantics | full style resolution is L | explicit per-cell props only + optional simple firstRow emphasis; warning when `tableStyleId` present |
| Hyperlink **annotations** | keep v1 surface small (color IS in scope, §3.g) | hyperlink-colored text, no link annot |
| Kerning/shaping/ligatures | pdf-typeset v1 is additive-advance | per-char advances |
| Notes pages | PDF is slides only | notes remain available via `Slide.notes` |

---

## 2. Current state (what pptspine has today)

Parser character: recursive-descent quick-xml walker, local-names only, unknown elements skipped
(`crates/ppt-parse/src/xml/slide.rs:753-771`), tolerant, never panics.

**Parsed today** (the floor ppt-render starts from):
- `sldSz` + slide order via rels with numeric fallback (`crates/ppt-parse/src/xml/presentation.rs:30-51`,
  `crates/ppt-parse/src/zip_pkg.rs:66-77`).
- `a:off`/`a:ext` → `Rect` in EMU, all-four-or-`None` (`slide.rs:209-242`).
- Text: paragraphs (`lvl`, `algn` — `slide.rs:338-342`), runs with exactly
  `text/font(a:latin)/size_pt/bold/italic/color(srgbClr only)` (`crates/ppt-core/src/model.rs:66-76`,
  verified) — nothing else.
- Tables: rows/cells/`gridSpan`/`rowSpan`/`hMerge`/`vMerge`/cell solidFill/row height
  (`slide.rs:508-627`).
- Pictures: `blip@r:embed` → media name → bytes (`slide.rs:681-705`, `zip_pkg.rs:99-109`), exposed via
  `Presentation.image_bytes()` (`crates/py-bindings/src/lib.rs:317-320`).
- Autoshapes: `prstGeom@prst` name, solid fill, stroke color (`slide.rs:180-197,246-301`).
- Groups: recursion only — `Shape::Group(Vec<Shape>)` carries **no rect/transform**
  (`crates/ppt-core/src/model.rs:34-45`, verified); `grpSpPr` is skipped wholesale (`slide.rs:91-92`,
  verified) ⇒ **grouped shapes sit in raw child coordinates; absolute positions are wrong today**.
- Notes text, OCR bridge, `to_text()`/`to_markdown()` (`crates/ppt-core/src/export.rs:9-44`).

**The critical callout.** slideLayout/slideMaster/theme parts are **never opened** — only their
basenames are read via rels (`zip_pkg.rs:113-127`, verified: `layout_name_for`/`master_name_for_layout`
return `basename(target)` and nothing else). `p:nvSpPr` (which holds `<p:ph type= idx=>`) is skipped in
`parse_sp` (`slide.rs:117-132`, verified: only `spPr`/`txBody` matched). Consequence: in real decks,
**title/body placeholders parse with `rect = None` and fully unstyled runs**
(`font=None, size_pt=None, color=None`) because their geometry and text styles live on the
layout/master/theme chain. A faithful PDF is impossible without §4.

**No drawing, rasterization, or font machinery exists anywhere in this workspace** — all of it comes
from pdf-typeset.

---

## 3. Parse-gap inventory (the core section)

Every row: verdict + evidence + effort + model growth needed. PARSED rows are in §2 and omitted here.

| # | Gap | Verdict | Evidence | Effort | Model growth (ppt-core) |
|---|---|---|---|---|---|
| a | slideMaster/slideLayout parts + placeholder geometry/style inheritance | **MISSING** (names only) | `zip_pkg.rs:113-127`; `nvSpPr` skipped `slide.rs:117-132`; `a:lstStyle` skipped `slide.rs:308-315` | **L** (largest item) | `Placeholder{kind, idx}` on shapes; `LayoutPart`/`MasterPart` IRs; `TextStyleLevels` (lvl 1–9) |
| b | theme1.xml: clrScheme + clrMap, fontScheme, color transforms | **MISSING** | only `a:srgbClr@val` handled, `parse_solid_fill` `slide.rs:246-276`; transforms explicitly skipped `slide.rs:258-266`; no `theme`/`schemeClr`/`lumMod` string in `crates/` (grep) | **L** (M+M+S) | `ColorSpec::{Srgb, Scheme{name, transforms}}` replacing terminal `Color` at parse level; `Theme{clr_scheme, font_scheme}` |
| c | `p:style` fillRef/lnRef/fontRef (theme-indexed shape styles) | **MISSING** | falls to skip in `parse_sp` `slide.rs:131` | **M** (needs b) | `StyleRef{idx, color}` ×3 on AutoShape |
| d | xfrm `rot`/`flipH`/`flipV` | **MISSING** | `xfrm`'s own attrs never read — caller consumes Start at `slide.rs:181` | **S** parse / **M** render math | `rot: i32` (1/60000°), `flip_h/flip_v: bool` on a new `Xfrm` |
| e | group `xfrm` + `chOff`/`chExt` remap | **MISSING — grouped coords wrong today** | `grpSpPr` skipped `slide.rs:91-92` (verified) | **M** | `Group{rect, child_rect, rot, flip_h, flip_v, children}` struct replacing bare `Vec<Shape>` |
| f | `bodyPr`: anchor/anchorCtr/insets/wrap/vert + `normAutofit@fontScale/lnSpcReduction` | **MISSING** | `parse_txbody` matches only `a:p` `slide.rs:304-325` | **S–M** parse; apply-stored-autofit **S** | `BodyProps` on TextFrame (OOXML inset defaults 91440/45720 EMU) |
| g | `pPr`: `lnSpc`/`spcBef`/`spcAft`/`marL`/`indent`/`buNone`/`buChar`/`buAutoNum`/`buFont`/`defRPr` | **PARTIAL** (lvl/algn only) | open-`pPr` branch skips all children `slide.rs:338-343` | **M** | `ParaProps` growth + `Bullet` enum + `def_rpr` |
| h | `rPr`: `@u`/`@strike`/`@baseline`/`@spc`, `a:ea`/`a:cs` typefaces (**critical for CJK**), `a:highlight`, `a:hlinkClick` color | **PARTIAL** | attrs beyond sz/b/i unread `slide.rs:388-392`; children beyond latin/solidFill skipped `slide.rs:443-448` | **S** each | run fields: `underline`, `strike`, `ea_font`, `cs_font`, `highlight`, `hyperlink` |
| i | `a:br` + `a:fld` inside paragraphs | **MISSING — silent text loss** | `parse_paragraph` matches only `pPr`/`r` `slide.rs:335-350` (notes.rs handles fld; slide.rs doesn't) | **S** | run-level `Break` marker + field runs with resolved text |
| j | `prstGeom` `avLst` adjust values | **MISSING** | skip right after grabbing `prst` `slide.rs:184` | **S–M** | `adjusts: Vec<(String, i64)>` on AutoShape |
| k | `custGeom` | **MISSING** | unhandled (skip) | **L** — v1 OUT (§1) | none in v1 |
| l | `a:ln` width/dash/cap/`headEnd`/`tailEnd` | **MISSING** | `parse_line_color` reads nested solidFill only `slide.rs:279-301` | **S** | `Stroke{color, width_emu, dash, caps, arrows}` |
| m | `gradFill` / shape-level `blipFill` / `noFill` distinction | **MISSING** (`noFill` indistinguishable from unset) | `parse_sppr` arms `slide.rs:180-188` | **S–M** | `Fill::{None, Solid, Gradient, Blip}` enum |
| n | picture `srcRect` crop + `stretch/fillRect` + `tile` | **MISSING** | `parse_blip_fill` reads only `blip` `slide.rs:686-695` | **S–M** | crop/stretch fields on Picture |
| o | slide background `bg`/`bgPr`/`bgRef` (slide→layout→master) | **MISSING** | walker fast-forwards to `spTree` `slide.rs:40-52`; no `bg` string in crate (grep) | **M** (`bgRef` needs b) | `Slide.background: Option<Fill>` |
| p | `tblGrid` column widths | **MISSING — blocks absolute cell x layout** | `parse_table` matches only `tr` `slide.rs:508-517` | **S** | `Table.col_widths: Vec<Emu>` |
| q | cell borders `lnL/lnR/lnT/lnB` + `tcPr` margins/anchor | **MISSING** | `parse_tcpr_fill` reads solidFill only `slide.rs:605-627` | **M** | `CellBorders` + margins/anchor on Cell |
| r | `tblPr` flags + `tableStyles.xml` | **MISSING** | never opened | **L** — v1 CUT (§1) | v1: firstRow flag only |
| s | non-table `graphicFrame` rect (charts/SmartArt/OLE) | **MISSING — rect lost** | `table.map(...)` returns `None` `slide.rs:494-500` | **S** | `Shape::Placeholder{rect, kind}` variant |
| t | connectors `p:cxnSp` | **DROPPED** | not in container match `slide.rs:71-93` (verified) | **S–M** | `Shape::Connector` (≈ AutoShape minus txBody) |
| u | `mc:AlternateContent` | **DROPPED WHOLESALE — shape loss in newer decks** | unknown name → skip `slide.rs:92` | **S** | none — walker descends into `mc:Fallback` |

---

## 4. Inheritance-resolution design (the heart)

### 4.1 The chain

For each slide the resolver loads (all via existing zip/rels machinery, `zip_pkg.rs`, `xml/mod.rs:27-66`):
its slideLayout part, that layout's slideMaster part, the master's theme part, plus `p:clrMap` (master)
and `p:clrMapOvr` (slide/layout).

**Placeholder matching** (slide ph → layout ph → master ph):
1. Match by `idx` when both sides carry one (absent `idx` ⇒ 0).
2. Else match by `type`, with the PowerPoint equivalence classes: `title ↔ ctrTitle`,
   `body ↔ subTitle ↔ (obj-ish placeholders holding text)`; `dt`/`ftr`/`sldNum` match by type only.
3. Layout → master falls back to type-only matching (a master has one title ph + one body ph).

**Geometry resolution:** first `a:xfrm` found walking slide ph → layout ph → master ph wins whole
(no per-field merge — matches PowerPoint). Non-placeholder shapes keep their own xfrm (required on them).

**Text-style merge**, per paragraph level `lvl` ∈ 1..9, later source wins **per attribute**:
```
master txStyles bucket (titleStyle | bodyStyle | otherStyle, chosen by ph kind)
  → master ph a:lstStyle          → layout ph a:lstStyle
  → slide txBody a:lstStyle       → paragraph a:pPr / a:defRPr
  → run a:rPr                     (direct formatting last, always wins)
```
Non-placeholder text boxes use `presentation.xml` `p:defaultTextStyle` + master `otherStyle` as the
base of the same merge. Bullets (`buChar`/`buAutoNum`/`buNone`) and indents (`marL`/`indent`) merge
through the identical chain — `buNone` at a nearer level suppresses an inherited bullet.

**Color resolution:** `a:schemeClr@val` → remap through `clrMapOvr` then `clrMap`
(`tx1→dk1`, `bg1→lt1`, …) → theme `clrScheme` RGB → apply child transforms **in document order**
(`lumMod`/`lumOff` in luminance space, `tint`/`shade` toward white/black, `alpha` → pdf-typeset
constant-alpha ExtGState, `satMod` best-effort). `sysClr` uses its `lastClr` attr. The same resolver
handles explicit `srgbClr` with transforms (today even alpha on explicit colors is dropped,
`slide.rs:258-266`).

**Font resolution:** `+mj-lt`/`+mn-lt`/`+mj-ea`/`+mn-ea` → theme `fontScheme` major/minor latin/ea;
`p:style > a:fontRef` idx picks major/minor; the resolved *name* is what ppt-render hands to
pdf-typeset's system-font resolver (TS-2). Shape `fillRef`/`lnRef` resolve at minimum the solid case
against the theme format lists; complex entries degrade to representative solid + warning.

### 4.2 Where it lives — recommendation

**Resolver in `ppt-parse` (new `src/resolve.rs` + `src/xml/{layout,master,theme}.rs`); resolved IR
types in `ppt-core` (new `resolved` module, plain data).** Rationale:
- Resolution needs zip parts + rels, which only ppt-parse has; the layout/master spTrees are parsed by
  **reusing the existing `slide.rs` walker** (same schema).
- ppt-core keeps its charter — "No IO/zip/XML" (`crates/ppt-core/src/lib.rs:2-5`) — by hosting only the
  resolved data types, exactly as it hosts `model.rs` today.
- Resolution belongs **before** rendering: `to_markdown`/`to_text` can later benefit (inherited bullets,
  theme fonts, placeholder text styles) without touching ppt-render. Putting it in ppt-render would
  trap the single hardest subsystem behind the PDF feature.
- Entry: a new explicit `ppt_parse::resolve(&ParsedPptx) -> ResolvedPresentation` — the existing
  `parse_path/parse_bytes` output and the to_text path stay byte-identical (minimal-change principle).

### 4.3 The "resolved slide" IR (handed to ppt-render)

`ResolvedPresentation{ slide_size, slides: Vec<ResolvedSlide> }`;
`ResolvedSlide{ background: Option<Fill>, shapes: Vec<ResolvedShape> }` in spTree z-order.
Every `ResolvedShape` carries: a **materialized rect** (placeholder geometry filled in), `rot`/flips,
terminal RGB colors (no scheme refs survive), resolved font names, fully-merged para/run/body props
(no `Option`-inheritance left), and for groups a `Transform` (offset/scale from `(child−chOff)·(ext/chExt)+off`,
plus rot/flip about center) with children kept nested — ppt-render accumulates transforms on recursion.

---

## 5. `crates/ppt-render` design

- **Deps:** `ppt-core` + `pdf-typeset` (git-pinned, §1). Nothing else. `#![forbid(unsafe_code)]`.
- **Entry:** `render_pdf(pres: &ResolvedPresentation, media: &BTreeMap<String, Vec<u8>>, opts: &RenderOptions) -> Result<ExportResult, PptError>`
  where `ExportResult{ bytes, warnings: Vec<ExportWarning> }` re-exports pdf-typeset's type;
  `RenderOptions` holds `font_map` overrides fed to the TS-2 resolver.
- **Slide → page:** one page per slide; size = `slide_size` EMU → pt via `Rect::to_points`/`emu_to_points`
  (`crates/ppt-core/src/geom.rs:36-57`); e.g. 16:9 `12192000×6858000` EMU ⇒ `960×540` pt. Build pages
  from scratch (pdf-typeset follows pdf-markdown's deterministic assembly, one content stream per page,
  fonts registered once — pdfspine `crates/pdf-markdown/src/render.rs:13-15,39-91`).
- **Z-order:** spTree document order is paint order (OOXML rule); `Vec<Shape>` already preserves it.
- **Shape dispatch:** `TextBox` → TS-5 absolutely-positioned text box (fixed rect, anchor, insets,
  wrap on/off, stored `normAutofit@fontScale` applied, rotation via `q cm … Q`, clip to shape);
  `Auto` → TS-6 preset-geometry path (fill → stroke → text box on top, in that paint order); presets
  outside the ~35-preset subset degrade to bounding-box rect + warning, text still rendered;
  `Picture` → decode + place with `srcRect` crop (clip + offset `cm`) and stretch/fillRect;
  `Table` → pdf-typeset table primitives: absolute cell x from accumulated `tblGrid` widths, row y from
  row heights (grown to fit content), per-cell borders as 4 lines, fills behind text;
  `Connector` → line/polyline with stroke props (arrowheads v1.x);
  `Group` → recurse with accumulated affine transform (nested `chOff`/`chExt` remaps compose);
  `Placeholder` (charts/SmartArt/OLE) → light-gray bounding box + warning.
- **Warning propagation:** ppt-render adds its own kinds (`UnsupportedChart`, `VerticalText`,
  `GradientDegraded`, `PresetFallback`, `FontSubstituted`, …) into the single `Vec<ExportWarning>`;
  never panics, never silently drops a shape that has a rect.
- **py-bindings wiring:** method on `PyPresentation`, which already holds `presentation` + `media` as
  two `Arc`s (`crates/py-bindings/src/lib.rs:257-261`); heavy work under `py.detach` like `open`
  (`lib.rs:409-415`); errors map through the existing exception hierarchy (`lib.rs:38-63`).

---

## 6. Python API (LOCKED)

```python
class Presentation:
    def to_pdf(self, *, font_map: dict[str, str] | None = None) -> bytes: ...
    def save_pdf(self, path: str | os.PathLike, *, font_map: dict[str, str] | None = None) -> None: ...
```

- Zero required args. `font_map` maps requested family → file path or family-name override, layered on
  top of the TS-2 substitution table (宋体→Songti SC etc.).
- Warnings surfaced via Python `warnings.warn`, **one per unique `ExportWarning` kind** (not per shape).
- Matches the established zero-arg export convention `to_text()`/`to_markdown()`
  (`python/pptspine/_core.pyi:49-50`, `crates/py-bindings/src/lib.rs:323-330`). Stubs added beside them;
  `save_pdf` implemented in the pure-Python layer as `Path(path).write_bytes(self.to_pdf(...))` or in
  Rust — either way the signature above is the contract.

---

## 7. Reuse map (落点地图 — don't re-investigate)

- New crate `crates/ppt-render` (deps: ppt-core + pdf-typeset git-pinned per §1); resolver in
  `crates/ppt-parse/src/resolve.rs`; binding in `crates/py-bindings`.
- **pdf-typeset Phase A deliverables consumed here** (specified in pdfspine `docs/PRD-NEXT.md` §10):
  **TS-2** system fonts — fontdb 0.23 + substitution table + per-char fallback + bundled Liberation/Noto
  last resort (`pdfspine crates/pdf-fonts/src/liberation.rs:1-52`); **TS-3** multi-face R/B/I/BI + TTC
  face index + usage-based glyph subsetter (today face index 0 is hardcoded,
  `pdfspine crates/pdf-edit/src/fontfile.rs:62`, and the **whole program is embedded verbatim**,
  `fontfile.rs:36-37,156-171`); **TS-5** absolutely-positioned TextBox (anchor/wrap/autofit-scale/
  rotation/clip); **TS-6** preset-geometry subset (~35 presets; arc→Bézier is the new math — full
  ellipses already exist, `pdfspine crates/pdf-edit/src/drawing.rs:205-249`).
- **Engine floor that already exists in pdfspine today:** `EmbeddedFont` Type0/Identity-H + always-on
  ToUnicode (`crates/pdf-edit/src/fontfile.rs:155-254,286-314` — guarantees extractable text for the
  read-back gate); greedy wrap with measurement==drawing per-char fallback
  (`crates/pdf-markdown/src/fonts.rs:16-17`, `layout.rs:310-394`); `Shape` path/paint builder with
  dash + even-odd (`crates/pdf-edit/src/drawing.rs:263-317`); full affine `Matrix` + arbitrary `cm`
  precedent (`crates/pdf-core/src/geom/matrix.rs:9-193`, `crates/pdf-edit/src/merge.rs:202-211`);
  image insertion (`crates/pdf-edit/src/image.rs:33,81`); drawings read-back for A/B asserts
  (`crates/pdf-edit/src/drawings.rs`).
- **EMU→pt:** `ppt_core::geom` — `Emu(i64)`, `EMU_PER_POINT = 12700`, `Rect::to_points()`
  (`crates/ppt-core/src/geom.rs:7-57`).
- **Read-back + scoring (acceptance stack):** pdfspine `Page.get_text()`
  (`pdfspine python/pdfspine/document.py:1813-1843`) + `get_text_words` coordinates;
  `pdfspine conformance/gt/score.py` (`content_scores` :152, `order_score` :198);
  `pdfspine conformance/gt/render_diff.py` (`ssim` :242-281, `_near_blank` :463-469);
  committed-refs precedent `pdfspine .github/workflows/ci.yml:189-194` (`--min-ssim 0.97`).
- **Fixtures:** extend `python/tests/conftest.py::_zip_pptx` (`conftest.py:160-166`) with
  export-oriented synthetic decks (16:9 + 4:3, master/layout/theme parts, placeholders,
  rotated/flipped/grouped shapes, prstGeom variety, tables with borders, cropped pictures, backgrounds).
  python-pptx exists only in the global anaconda python — authoring aid, never a test dep.
- **py-bindings template:** `to_text`/`to_markdown` methods (`crates/py-bindings/src/lib.rs:323-330`),
  two-Arc handle (`lib.rs:257-261`), `py.detach` heavy-work pattern (`lib.rs:409-415`).
- **⚠ TRAP:** pdfspine `insert_textbox` (`crates/pdf-edit/src/text.rs:238`) **drops overflow lines
  silently** — slide text must go through TS-5's own wrap, never `insert_textbox`.
- **⚠ TRAP:** until the TS-3 subsetter lands, fontdb-resolved CJK TTCs embed the **entire collection**
  (`fontfile.rs:36-37,156-171` — Songti.ttc ≈ 90 MB per PDF). Do not GA before TS-3.
- **⚠ TRAP:** grouped coordinates are wrong **today** (`slide.rs:91-92` skips `grpSpPr`) — never trust
  the rect of any shape inside a `Group` until B-5 lands; gate B-2 fixtures must avoid groups.
- **⚠ TRAP:** charts vanish **including their rect** (`slide.rs:494-500`) — the placeholder-box
  degradation needs the B-3 parse fix first; it cannot be done render-side.
- **⚠ TRAP:** `Tw` word spacing does not apply to Identity-H 2-byte codes — justification must
  redistribute frag x offsets (pdfspine `crates/pdf-edit/src/text.rs:44-48` treats Justify as Left).
- **⚠ TRAP:** `../pdfspine` is **READ-ONLY from this repo** (CLAUDE.md 铁律). All engine gaps route to
  Phase A tasks in pdfspine's own PRD; pptspine only bumps the pinned rev.
- **⚠ TRAP:** determinism is **per font environment** once system fonts resolve — never write
  byte-exact cross-machine golden tests; coordinate gates use 1 pt tolerances instead.
- **⚠ TRAP:** the LibreOffice oracle is **local-only advisory** — verified present at
  `/Applications/LibreOffice.app/Contents/MacOS/soffice` (25.2.1.2), `--headless --convert-to pdf`
  (impress_pdf_Export). Never in CI; SSIM advisory band 0.80–0.90.

---

## 8. Phased plan (B-1..B-11)

Sequencing principle: ship visible results early — explicit-geometry slides first; the inheritance
chain (B-8/B-9) lands later but **before GA**. Each task: why · effort · engine prereq · green condition.

- **B-1 · Scaffold + blank-page export + Python wiring.** New `crates/ppt-render` with pinned
  pdf-typeset dep; `to_pdf`/`save_pdf` on `PyPresentation` (thin); CI builds the git dep exactly as the
  ocrspine precedent (`release.yml:10-16`). Effort **M**. Prereq: pdf-typeset scaffold (TS-1).
  **Green:** pytest exports a deck to non-empty PDF bytes; pdfspine reads it back with
  page count == `slide_count` and every `Page.rect` == `slide_size_points` (`_core.pyi:44`) for both
  16:9 (960×540 pt) and 4:3 fixtures; fmt/clippy `-D warnings`/tests green on the workspace.
  · **状态**: done。
- **B-2 · Explicit-geometry text boxes.** Map `TextFrame` with explicit rect + today's five run attrs
  onto TS-5 boxes with TS-2/TS-3 fonts. Effort **M**. Prereq: TS-2, TS-3, TS-5.
  **Green:** single-textbox deck exports; `get_text_words` bbox within **1 pt** of the EMU-derived rect
  (+ inset defaults); token-F1 & order ≥ **0.99** vs `to_text()` (minus separators/notes) scored with
  `score.py`; raster not `_near_blank`; exactly one FontFile2 per used face (subsetted).
  · **状态**: done。
- **B-3 · Parse loss-fix batch.** `a:br`/`a:fld` (§3.i), `mc:AlternateContent` Fallback descent (§3.u),
  `p:cxnSp` (§3.t), non-table graphicFrame rect capture (§3.s), `ea`/`cs` fonts + `u`/`strike` (§3.h),
  `ln` width/dash (§3.l), `tblGrid` widths (§3.p). Effort **M** (many S items). Prereq: none (parse-only).
  **Green:** Rust parse tests assert each new field from synthesized XML; a deck with `a:br` round-trips
  both lines through `to_pdf` → `get_text`; a cxnSp deck yields a visible stroked line (drawings
  read-back); CJK run with only `a:ea` typeface resolves a CJK font (no `?` glyphs in read-back).
  · **状态**: done。
- **B-4 · Drawing fidelity: autoshapes + pictures.** prstGeom subset via TS-6 + `avLst` adjusts (§3.j),
  `rot`/`flipH`/`flipV` (§3.d), `Fill` enum with `noFill` (§3.m), stroke props, picture `srcRect`/
  stretch (§3.n), chart/SmartArt placeholder boxes. Effort **L**. Prereq: TS-6.
  **Green:** rotated-45° textbox's `get_text_words` center within 1 pt of the rect center; images
  survive via `extractIMGINFO`; roundRect-with-adjust raster differs from default-adjust raster
  (SSIM < 1.0 between them, both non-blank); unknown preset emits exactly one `PresetFallback` warning.
  · **状态**: done。
- **B-5 · Group transforms.** `Group{rect, child_rect, …}` + `(child − chOff)·(ext/chExt) + off`
  nested remap in ppt-render (§3.e). Effort **M**.
  **Green:** a grouped-and-scaled textbox deck and its pre-flattened ungrouped twin produce
  `get_text_words` coordinates equal within 1 pt; nested-group fixture included.
  · **状态**: done。
- **B-6 · Text-box + paragraph fidelity.** `bodyPr` anchor/insets/wrap/`normAutofit@fontScale`
  applied-as-stored, `spAutoFit` = no-op (§3.f); `pPr` lnSpc (multiple + exact)/spcBef/spcAft/
  marL/indent/bullets/`defRPr` (§3.g). Effort **L**. Prereq: TS-5 props surface.
  **Green:** bottom-anchored box's last-line word baseline within 1 pt of `rect.bottom − bIns`;
  `buChar` bullet glyph and `buAutoNum` "1." both present in read-back text; `lnSpc spcPct=200%`
  doubles inter-line word-y delta within 5%.
  · **状态**: done(spcPct 段距、纵排降级 + 告警;**重算式 autofit** 已接——`normAutofit`
  生效但未存 fontScale 时,用引擎 TS-10 `measure_text_box` 逐档收缩 fontScale/lnSpcReduction
  至内容落框,stored fontScale 路径不回归)。
- **B-7 · Tables.** Absolute cell x from `tblGrid`, borders as per-side lines, fills, margins, anchor
  (§3.p/q). Effort **M**.
  **Green:** each cell's first word x within 1 pt of accumulated `gridCol` widths + margin; border
  lines present in `get_drawings` read-back at grid coordinates; merged-cell fixture renders no
  interior border inside the span.
  · **状态**: done(表格网格 / 逐边框线 / 填充 / tcPr 内边距与锚定;**按内容自适应增高**已接——
  用引擎 TS-10 `measure_text_box` 算每行内容高,取 `max(基线行高, 内容高)`,rowSpan 高度跨行均摊)。
- **B-8 · Theme subsystem.** Theme part parse, `clrScheme` + `clrMap`/`clrMapOvr`, transform math,
  `fontScheme` (+ea), `p:style` fillRef/lnRef/fontRef solid resolution (§3.b/c, §4.1). Effort **L**.
  **Green:** golden table of (schemeClr, transforms) → RGB passes within ±2/255 (values harvested from
  a PowerPoint/LO-rendered reference, sampled via `get_pixmap`); `+mn-lt` run resolves to the theme
  minor font; alpha fill produces an ExtGState (drawings read-back).
  · **状态**: done(theme / clrScheme / clrMap / clrMapOvr / transforms / fontScheme + fillRef/lnRef/fontRef solid)。
- **B-9 · Placeholder inheritance chain.** `ph` capture, layout/master part parse (reusing the
  `slide.rs` walker), `resolve()` + `ResolvedPresentation` (§4). Effort **L** (the largest item).
  **Green:** fixture where the slide title has **no xfrm and no rPr**: exported word bbox matches the
  layout's title rect within 1 pt and the read-back font size equals the master `titleStyle` lvl1 size
  (asserted at ResolvedSlide level too); lvl2 body bullet inherits the master bullet char; token-F1 &
  order ≥ 0.99 on the full placeholder deck.
  · **状态**: done(占位符 `ph` 捕获 + layout/master 部件解析 + `resolve()` 终态 IR)。
- **B-10 · Backgrounds.** `bg`/`bgPr` solid + picture, chain slide→layout→master, `bgRef` via theme;
  gradient degrades (§3.o). Effort **M**. Prereq: B-8, B-9.
  **Green:** corner-pixel sample of `get_pixmap` equals the background RGB within ±2/255; gradient bg
  emits one `GradientDegraded` warning and a non-blank page.
  · **状态**: done(背景 slide→layout→master + `bgRef`;gradient 降级告警)。
- **B-11 · GA hardening + family-stack gate.** Warning-surfacing audit (one `warnings.warn` per unique
  kind), `font_map` override test, README capability rows, local real-deck corpus (gitignored)
  spot-check. Effort **M**.
  **Green (the family-stack gate):** on the full synthetic fixture matrix — (1) CI-blocking read-back:
  token-F1 & order ≥ 0.99 (`score.py` `content_scores`:152 / `order_score`:198) vs `to_text()` minus
  separators/notes; (2) CI-blocking structural: page count == slide_count, `Page.rect` ==
  `slide_size_points`, `get_text_words` EMU→pt coordinate asserts (1 pt tolerance), image survival via
  `extractIMGINFO`, no page `_near_blank` (`render_diff.py:463-469`); (3) local-only advisory:
  LibreOffice oracle SSIM in the 0.80–0.90 band via `render_diff.py` `ssim`:242-281 (never CI);
  (4) committed `.ssimref` refs at `--min-ssim 0.97` per pdfspine
  `.github/workflows/ci.yml:189-194` — a **self-render** regression gate (our own
  `to_pdf()` raster vs committed grayscale references), not the LO oracle.
  · **状态**: done(告警逐种类上浮、`font_map` 覆盖、README 能力表;门(4) 落地——
  `scripts/ssim_baseline.py` + `python/tests/ssim_refs/*.ssimref` + `test_ssim_gate.py`,
  固定字体 runner 上 `--min-ssim 0.97` 强制,`--make-references` 有意重生成)。

---

## 9. Risks & mitigations

1. **Inheritance-chain correctness vs PowerPoint semantics** (top risk). idx-vs-type matching corner
   cases, `ctrTitle`/`subTitle` equivalences, `lstStyle`-vs-`txStyles` precedence are folklore-adjacent.
   *Mitigation:* assert at the **ResolvedSlide IR level** (unit-testable without rendering); one
   synthetic deck per rule; A/B against the LibreOffice oracle on single-feature decks; python-pptx
   (authoring aid only) to generate reference decks whose effective styles are known.
2. **Theme color transform math.** `lumMod`/`lumOff` operate in luminance space, `tint`/`shade` have
   gamma subtleties; exact PowerPoint output is not normatively specified.
   *Mitigation:* golden (input, transforms) → RGB table harvested from rendered pixels; ±2/255 gate
   tolerance (B-8); document residual deltas as advisory.
3. **Autofit text overflow.** `normAutofit@fontScale` is baked at last save; stale values overflow.
   *Mitigation:* v1 applies stored scale + clips to the shape (matches pptx viewer behavior); emit a
   warning when laid-out height exceeds the box by >5%; recompute-from-scratch autofit stays out (L).
4. **Engine API churn during Phase A.** pdf-typeset is being built concurrently.
   *Mitigation:* pinned `rev` bumps are deliberate, reviewed events; ppt-render stays a thin mapper;
   B-1/B-3 (parse work) proceed independent of engine state.
5. **Font-environment variance (CI vs dev).** System font resolution differs per machine.
   *Mitigation:* bundled Liberation/Noto last resort guarantees non-blank output everywhere; gates use
   1 pt metric tolerances, not byte equality; `.ssimref` refs generated on a pinned-font runner only.
6. **Group-remap regressions.** The B-5 affine fix changes every grouped deck's output.
   *Mitigation:* grouped-vs-ungrouped-twin equality gate (B-5) plus nested-group fixture locked in CI.

---

*Sources: pptspine research findings 2026-07-02 (spot-verified against `slide.rs`, `zip_pkg.rs`,
`model.rs`, `Cargo.toml`, `conftest.py`, `py-bindings/src/lib.rs`); pdfspine engine findings 2026-07-02;
pdfspine `docs/PRD-NEXT.md` §9 (format precedent) and §10 (Phase A engine spec, referenced by name).*
