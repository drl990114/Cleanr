# TUI interface verification — 2026-09-08

Status: implemented in the local development checkout; not released.

## Scope

Home, Scan, Usage, Restore, Languages, Rules, Plugins, Tasks, command palette,
help, filters, and confirmations share the revised layout and presentation.
The root applies one responsive gutter and a 220-column content limit. At 88
terminal columns, lists retain a 32–56-column detail pane; smaller terminals
use a full-width detail overlay. Focus changes preserve cell positions and
wrapping. Lists, the category filter, and the palette reuse Ratatui Table column
constraints; only visible rows are formatted. No project dependency was added.

Routine header summaries and repeated fields are suppressed. Technical metadata
is disclosed with `i` in details; the choice lasts for the session, per page.
All list pages support detail scrolling, and the scrollbar uses a reserved
gutter. `p` edits an inline query above the scan list. Existing selection keys,
filtered/global bulk selection, query cancellation, and background projections
remain in place. Focused details consume list action keys. Confirmations start
at Cancel, size to their content, and require complete evidence and visible
buttons before submission. English and Simplified Chinese help and usage
documentation are synchronized.

Public Theme/TuiOptions, CLI/configuration contracts, serialized evidence,
authorization, final validation, Trash, and restore behavior are unchanged.

## Layout and interaction checks

The new `ui_` tests exercise English and Simplified Chinese with both built-in
themes at 40×12, 60×20, 80×24, 87×24, 88×24, 120×32, and 180×48:

- Scan cell coordinates verify check/path alignment, right-aligned mixed units,
  Chinese/emoji/combining-character truncation, and the final visible row.
- All seven list pages verify stable wide-pane geometry, Tab/Shift+Tab, scrolling,
  disclosure, retained list position, layered help/Esc, and blocked underlying
  actions. Repeated `i` does not toggle disclosure repeatedly.
- Query cancellation restores the previous filter; selection accumulates across
  filters. Empty and read-only results retain their selection restrictions.
- A narrow-path regression keeps `project-0` and `project-1` distinguishable when
  both end in `node_modules`. Truncation retains the beginning and end by display
  width; complete paths remain in details.
- Confirmation checks cover Cancel focus/contrast, restore confirmation, all-
  selected review, and refusal to submit when the full dialog cannot fit.

Focused TUI tests, all six i18n unit tests, `cargo fmt --all -- --check`, scoped
Clippy with all targets/features and `-D warnings`, documentation `pnpm typecheck`,
and `git diff --check` passed. No build or complete workspace test suite was run.

## Local performance evidence

The existing unoptimized 120×40 TestBackend benchmarks were run before and
after the change. Each interaction uses 100 samples after warmup. Measurements
cover key handling and TestBackend draw completion, not physical terminal paint.

| Candidates | Measurement | Before P95 | Final run P95 |
| ---: | --- | ---: | ---: |
| 10,000 | Down | 2.942 ms | 7.510 ms |
| 10,000 | Space | 2.957 ms | 3.183 ms |
| 10,000 | Confirmation | 3.197 ms | 3.200 ms |
| 100,000 | Down | 2.928 ms | 2.885 ms |
| 100,000 | Space | 2.965 ms | 2.940 ms |
| 100,000 | Confirmation | 3.203 ms | 3.098 ms |

Timing varied across local runs: earlier post-change 10,000-candidate Down P95
was 2.797–2.958 ms, and 100,000-candidate confirmation P95 ranged from 3.098 to
7.420 ms. These measurements pass the existing 50 ms interaction threshold;
they do not establish a universal latency improvement. The 200-frame render-only
run measured mean 2.745 ms, P95 2.921 ms, max 2.998 ms, compared with the initial
mean 2.802 ms and P95 3.020 ms, below the existing 16 ms draw threshold.

Twenty query/view-switch cycles retained the same plan pointer and one shared
scan index. In the final runs, RSS grew from 55,680 to 55,712 KiB for 10,000
candidates and from 367,200 to 367,216 KiB for 100,000 candidates. This is evidence
for the tested sequence, not a general memory bound.

## PTY verification

The existing `interactive_terminal_fixture` generated eight temporary project
candidates. A 120×32 session exercised scan cancellation, completed scan, detail
expansion and paging, inline filtering, hidden selections, all-selected review,
usage browsing, cancellation, and quit. A second capture resized the live PTY
from 120×32 to 40×12 and verified the detail overlay, list, default-Cancel dialog,
confirmation cancellation, and terminal restoration. No cleanup or restore was
submitted. PNG previews were rendered locally from captured ANSI cells using
the repository demo recorder's pyte/Pillow approach; they are not native terminal
screenshots or production-package/cross-platform evidence.
