# cleanr-tui

Terminal user interface for cleanr.

This crate runs the interactive ratatui-based application: event handling,
screen rendering, command palette, and background scan coordination.

The source is split by responsibility:

- `app/` owns application state, input handling, navigation, task events, and
  action orchestration.
- `effects/` is the boundary for threads, plugin/runtime loading, cleanup and
  restore execution, configuration persistence, and manifest I/O.
- `views/` contains rendering only, split by screen and overlay.
- `commands/` contains command-palette filtering and command metadata helpers.
- `terminal.rs` owns raw-mode setup, the event loop, and terminal restoration.

Candidate and usage views share immutable scan snapshots. `r`/`u` reuse them;
`/plan`, `/usage`, and `s` request explicit background work. `projection.rs`
prepares category/search rows and stable sort indices on workers. Query and
scan revisions fence stale responses; selection counts update incrementally.
The frame path formats only visible candidate rows. No new search dependency is
needed: normalized substring matching and the existing Ratatui/Crossterm APIs
cover the interaction. Ratatui's locked rendered-line-info feature supplies
exact wrapping bounds for details and fail-closed confirmation visibility.

`terminal.rs` draws only after visible state changes or active-task animation.
It batches navigation for at most 32 events or 4 ms, preserving action-key and
resize boundaries. `/tasks` shows bounded handler, draw, task-commit, and
input-read-to-frame duration samples. These are local diagnostics, not telemetry.
`run_with_services` optionally accepts an asynchronous update notice receiver;
existing `TuiOptions` and entry points remain compatible.

Run focused interaction tests with `cargo test -p cleanr-tui interaction_ --lib`.
For a manual PTY using temporary fixture data, run
`cargo test -p cleanr-tui interactive_terminal_fixture -- --ignored --nocapture`.
The fixture requires a terminal and exits with `q`.
