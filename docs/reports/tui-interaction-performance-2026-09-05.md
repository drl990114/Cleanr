# TUI interaction and performance verification — 2026-09-05

Status: implemented in the local development checkout, not released.

## Delivered behavior

- The update request runs in a background thread with a 10-second HTTP timeout.
  Home displays its notice separately from task status and errors. Existing TUI
  entry points and `TuiOptions` remain compatible.
- `r` and `u` reuse the current snapshot and preserve per-view focus/offset.
  `s`, `/scan`, and `/usage` explicitly re-scan; `/plan` rebuilds on a worker.
  Usage computation and restore-history I/O also run on workers.
- `p` finds paths with 100 ms debounce, Unicode lowercase matching, and search-only
  slash normalization. Enter applies; Esc restores the previous query. `o` chooses
  original plan, size descending, or path ascending order. `v` shows selected items.
  Query, category, and selected-only conditions intersect; sorting does not reorder
  the cleanup plan. Selected-only removal focuses a neighbor.
- `a`/`%` act on the entire visible projection, and `Shift+A` on the global plan.
  Empty projections are no-ops for current-item/current-projection selection.
  Global count, bytes, and hidden selections remain visible.
- `Tab` focuses scrollable evidence. Below 88 columns it opens a full-width overlay;
  the full width also prevents CJK glyphs underneath from hiding its border.
  Details foreground risk and recommendation, retain the full path, and keep
  Enter/Space from changing selection. Help scrolls too.
- Scope/age and distinct empty states explain the current results. Confirmation
  starts at No, counts Review selections, and offers `v` to inspect all selected
  items before re-confirming. Incomplete confirmation layouts disable submission.
- Cleanup and restore freeze selection, show stages and processed counts, and
  advance counts only after the outcome journal is durable. Cleanup results show
  count, size, path context, and a `z` restore-history action.

## Implementation and reuse

The existing Ratatui/Crossterm UI, shared task workflow, standard channels/Arc,
and the existing experimental `ignore` walker are reused. No Rust dependency
was added. Ratatui 0.29's `unstable-rendered-line-info` feature supplies the
renderer’s wrapping calculation; the locked version and layout tests constrain
that feature’s upgrade risk. A dependency is unnecessary for substring search.

Workers prepare category rows and stable sort indices. Explicit scan/query
revisions and cancellation fence stale results; completed projections preserve
Scan focus even when another view is open. Selection counters update
incrementally. Frames format visible rows and reuse immutable data. The event
loop draws only when visible state changes or a task animates, batching only
navigation for at most 32 events or 4 ms. Rules, evidence, and planning now have
cooperative cancellation checks; partial work is never published as a new plan.

Protection, overlap resolution, trust checks, final validation, authorization,
system Trash, journal order, and restore safeguards remain in their owning crates.
No schema version or cleanup eligibility policy changed.

## Local evidence

Environment: macOS, Rust 1.98, unoptimized test executables, 120×40 TestBackend.
Timing excludes compilation. Candidate fixtures have old modification timestamps
and assert that the requested number of candidates is actually visible. The old
render fixture had missing timestamps and could measure an empty projection;
its timing cannot serve as a valid large-list baseline.

Each interaction has 100 samples after warmup. P95 below is key handling plus
TestBackend draw completion, not OS key-queue delay or physical terminal paint.

| Candidates | Navigation P95 | Single selection P95 | Confirmation P95 |
| ---: | ---: | ---: | ---: |
| 10,000 | 2.862 ms | 2.935 ms | 3.142 ms |
| 100,000 | 2.914 ms | 2.939 ms | 3.177 ms |

The 10,000-row render-only benchmark (200 frames) measured mean 2.780 ms,
P95 2.933 ms, max 3.255 ms. These meet the local 16 ms draw and 50 ms interaction
targets. The cancellation-feedback regression also requires a rendered response
within 100 ms; this is a UI feedback bound, not an interruptible-OS-call guarantee.

After 20 alternating queries and Home/Usage/Scan switches:

| Candidates | Starting RSS | Ending RSS | Process peak RSS |
| ---: | ---: | ---: | ---: |
| 10,000 | 54,688 KiB | 54,720 KiB | 56,082,432 bytes |
| 100,000 | 368,832 KiB | 368,896 KiB | 377,798,656 bytes |

Peak RSS was measured with `/usr/bin/time -l` around the test executable. The
same plan pointer and one shared scan index survived all 20 switches. This
supports bounded retention for the tested sequence; the 100,000-candidate
fixture still carries substantial evidence and path data in memory.

A PTY smoke test exercised current source via `interactive_terminal_fixture`,
using generated temporary candidates and an update receiver held pending:
first complete Home text arrived in 52.46 ms; an idle 1.15-second interval emitted
zero bytes. Scan, query, details, usage reuse, review, No-default confirmation,
confirmation cancellation, help, quit, and terminal restoration passed. This is
PTY output evidence, not a production-package or cross-platform rendering claim.

### Scanner worker comparison

The same generated fixture contained 100 directories × 100 files (1 KiB each).
Five warmed rounds per backend produced the same report fingerprint
`d57fa76ad4cf41c3`, entry count, bytes, and errors.

| Workers | Median | P95 | Process peak RSS |
| ---: | ---: | ---: | ---: |
| 1 | 67 ms | 69 ms | 10,895,360 bytes |
| 2 | 60 ms | 61 ms | 12,156,928 bytes |
| 4 | 50 ms | 51 ms | 14,483,456 bytes |

The default remains one worker. Four workers exceeded the 1.25× serial peak-RSS
limit (1.33×). Two workers improved P95 by about 12% on this single warm fixture;
that is insufficient evidence for a cross-platform default change.

## Verification and reproduction

Relevant TUI checks cover query rollback, stale generations, off-screen focus,
sort/selection separation, empty/global selection, read-only results, frozen
operations, cancellation, restored view positions, and notices that preserve
errors. The layout matrix covers English/Chinese and dark/light themes at
120×40, 80×24, 60×20, and 40×12, plus a smaller confirmation rejection case.
Existing command, category, scan, usage, cleanup, restore, and Home regressions
were also exercised. Core evidence/plan, workflow cancellation, i18n, and fake
executor recovery tests passed, including journal-failure progress behavior.

Relevant crate Clippy checks used all targets/features and `-D warnings`.
`cargo fmt --all -- --check`, documentation `pnpm typecheck`, the 28-file MDX
syntax check, and `git diff --check` passed. No build, full workspace test suite,
real Trash operation, release, or unrelated README edit was performed.

Reproduce the focused interaction measurements:

```sh
CLEANR_BENCH_CANDIDATES=10000 cargo test -p cleanr-tui --locked interaction_performance_large_snapshots -- --ignored --nocapture
CLEANR_BENCH_CANDIDATES=100000 cargo test -p cleanr-tui --locked interaction_performance_large_snapshots -- --ignored --nocapture
cargo test -p cleanr-tui --locked scan_view_render_performance -- --ignored --nocapture
```

For peak RSS, rerun the printed test executable directly under `/usr/bin/time -l`.
See `docs/docs/development.md` for scanner and interactive PTY fixture commands.
A production build, packaged terminal/IME checks, and Linux/Windows measurements
remain release gates, outside this local no-build verification.
