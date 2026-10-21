# Compatibility acceptance matrix

This is the quality and validation plan for TraceDraw. It measures whether a
user's work can be created, edited, saved, reopened and exported correctly.
The inventory in [features.md](features.md) is the implementation checklist,
not evidence of cross-application equivalence or production readiness.

## Evidence levels

- **Inventory**: a feature is claimed as available in the product.
- **Unit**: the implementation has a reproducible test.
- **Interaction**: the actual user action is tested, including modifiers,
  selection, undo/redo and cancellation.
- **Round-trip**: an editable saved file is reopened and its structure compared.
- **Independent output**: an exported file is inspected by another tool.
- **Production**: dimensions, fonts, colour profiles and printed/exported
  results meet the agreed tolerances with real sample files.

A row is **verified** only after its required evidence is linked to a test,
fixture or reproducible test report. Until then it is **not verified** even if
features.md says `works`. Do not infer percentages from the inventory totals.

## Prioritised cases

| ID | Priority | Workflow | Required verification | Current evidence |
|---|---|---|---|---|
| VEC-01 | P0 | Union, trim, intersection of curves | Preserve editable cubic segments, small node count, holes and correct topology | **Open**: `crates/core/src/shaping.rs` flattens curves to polylines |
| VEC-02 | P0 | Select, move, resize and rotate grouped objects | Pointer/keyboard modifiers, nesting, correct hit testing and a single undo step | **Open**: interaction reports need reproduction |
| VEC-03 | P0 | Edit a clipping frame and its contents | Frame styling, selection and edits inside the frame, undo and save | **Open**: real-file/browser issue |
| VEC-04 | P0 | Pan, zoom and select on a dense document | Benchmark p50/p95 for navigation and render latency, on-screen correctness | **Open**: reported slow interactions |
| CDR-01 | P0 | Open recent CDR documents | Objects, pages, hierarchy, text and embedded bitmaps compared with source | **Partial**: uncovered records documented in `blueprint-gaps.md` |
| CDR-02 | P0 | Save, reopen and resave CDR | No silent loss of supported objects, fonts, colour and dimensions | **Partial**: version coverage varies |
| DOC-01 | P0 | Text layout and conversion to curves | Exact text bounds, line breaks, original font behavior and outlines | **Not verified** on representative production fixtures |
| CLR-01 | P0 | ICC, RGB/CMYK and spot colours | Profile preservation, conversions, separations and export consistency | **Not verified** on production fixtures |
| OUT-01 | P0 | Export PDF/X, SVG and EPS | Independent rendering, printable dimensions, paths, ink values, transparency | **Not verified** on production fixtures |
| OUT-02 | P1 | Print actual-size vector output | Print settings, marks, separations and printer driver interaction | **Partial**: system printing currently uses the system PDF viewer |
| UI-01 | P1 | Keyboard shortcuts and modal commands | Reproducible key sequences, menu availability, no accidental actions | **Inventory only** |
| UI-02 | P1 | Toolbars, panels and property bars | State persistence, selection-specific controls and window scaling | **Inventory only** |
| TXT-01 | P1 | Complex text and embedded fonts | Mixed formatting, paragraphs, path text, missing-font handling | **Partial** for external-file coverage |
| AUT-01 | P1 | Scripted production tasks | Object-model behavior, errors, undo/redo and script fixture suite | **Inventory only** |
| IMG-01 | P2 | Trace bitmap to editable vector | Small node count, precise curves, transparent backgrounds and colour reduction | **Inventory only** |
| IMG-02 | P2 | Generate and edit vector artwork | Document editability, predictable errors and fallback without a service | **Inventory only** |
| EXT-01 | P2 | Additional exchange formats | Import/export of actual files and honest warnings for skipped content | **Partial** per format |

## P0 release gate

A P0 scenario is complete when:

1. Its functional requirements have runnable tests and reproducible inputs.
2. Interactive behavior is checked on an actual editor build, not inferred
   from a function existing in the source code.
3. File round-trips compare structure, object dimensions and style attributes;
   each intentionally unsupported construct produces a visible warning.
4. Exported vectors remain vector when the operation claims to preserve
   vectors. Production PDFs are checked using independent readers.
5. At least one real production artwork and one deliberately difficult fixture
   exercise the full workflow, including reopen and re-export.
6. There is no known P0 regression for the scenario.

## Native format and sample files

Maintain a corpus grouped by format version, object type and expected
rendering. Use original, donated or openly licensed test documents only.
Record the creator version, expected feature coverage, known unsupported
elements and anonymised reproduction steps. Follow the clean-room policy
in `AGENTS.md` and `CLEANROOM.md`. Never import proprietary program code.

Store small freely redistributable fixtures alongside their expected
structural assertions. Keep licensed or sensitive production assets outside
the public repository. No test is accepted solely because a file opens
without an error.

## Baseline from 2026-10-21

- The inventory has 215 `works` rows, five `partial` rows, one
  `missing` row and four `not applicable` rows, plus an icon assessment.
- These numbers describe *declared* coverage, not independent verification.
- The external `.cdr` coverage remains partial by the inventory's own
  admission. Older-format writing and many newer effects need fixture checks.
- Existing automated UI tests are defined in the app crate but were excluded
  from the original `test` job. CI must exercise them before acceptance.
- The directories `tests/visual/expected/`, `tests/cdr/`,
  `crates/render/benches/` and `crates/io/benches/` are not in this
  baseline. Acceptance claims dependent on them are future gates, not
  presently enforced checks.

See [acceptance.md](acceptance.md), [roadmap.md](roadmap.md) and
[blueprint-gaps.md](blueprint-gaps.md).
