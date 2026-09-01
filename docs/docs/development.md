---
description: Build, test, lint, and document Cleanr as a contributor.
---

# Development

## Prerequisites

- Rust 1.94.1 or a compatible newer toolchain
- Node.js 20 or later for the documentation site
- pnpm 10

## Build the workspace

Build the workspace:

```bash
cargo build
```

Build the release binary:

```bash
cargo build --release
```

The CLI binary is `target/release/cleanr`.

## Run the same checks as CI

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo build --workspace --all-targets --all-features --locked
```

Validate generated JSON Schemas:

```bash
cargo run --locked -p cleanr-cli -- plugin schema manifest >/dev/null
cargo run --locked -p cleanr-cli -- plugin schema index >/dev/null
cargo run --locked -p cleanr-cli -- plugin schema rules >/dev/null
cargo run --locked -p cleanr-cli -- plugin schema language >/dev/null
cargo run --locked -p cleanr-cli -- plugin schema config >/dev/null
```

## Measure local scan performance

The ignored filesystem benchmark scans only the root you provide and prints
aggregate timing, entry, error, and byte counts. It does not print individual
paths or run during the normal test suite.

```bash
CLEANR_BENCH_ROOT=/path/to/local/fixture \
CLEANR_BENCH_ROUNDS=5 \
CLEANR_BENCH_WORKERS=1 \
cargo test -p cleanr-fs --locked --test scan_performance -- \
  --ignored --nocapture
```

Compare the same fixture, filesystem state, build profile, and warm/cold-cache
condition before and after a scanner change. Do not interpret a developer
machine result as a cross-platform release claim. Worker counts above `1`
exercise an internal experimental backend, not a user configuration setting.
Do not expose or enable it by default unless repeated same-fingerprint runs show
a material P95 improvement and independently measured peak RSS stays within
`1.25x` of the serial baseline. On macOS, run the compiled test executable
directly under `/usr/bin/time -l` so Cargo and compiler memory are excluded;
the benchmark's `rss_after_kib` is only an after-scan snapshot, not peak RSS.

For the in-memory evidence, plan, and JSON serialization phases, use the
synthetic ignored benchmark. Its paths are generated fixture names rather than
local filesystem paths.

```bash
CLEANR_BENCH_ENTRIES=100000 \
CLEANR_BENCH_ROUNDS=5 \
cargo test -p cleanr-core --locked --test pipeline_performance -- \
  --ignored --nocapture
```

The ignored TUI benchmark measures only warmed `TestBackend` draw calls after
building a large synthetic candidate set. It prints mean, P95, and maximum frame
time without imposing a machine-specific pass threshold:

```bash
CLEANR_BENCH_CANDIDATES=10000 \
CLEANR_BENCH_FRAMES=200 \
cargo test -p cleanr-tui --locked \
  scan_view_render_performance -- --ignored --nocapture
```

## Run the documentation site

```bash
cd docs
pnpm install
pnpm start
```

The development server is available at `http://localhost:3000/` by default.

Before submitting documentation changes:

```bash
pnpm typecheck
pnpm build
```

## Keep English and Chinese in sync

- English source pages live in `docs/docs/`.
- Simplified Chinese pages live in
  `docs/i18n/zh-Hans/docusaurus-plugin-content-docs/current/`.
- Shared UI strings live in the locale JSON files under
  `docs/i18n/zh-Hans/`.

After changing translated React text, navbar labels, footer labels, or sidebar
categories, regenerate translation keys:

```bash
pnpm docusaurus write-translations --locale zh-Hans
```

Then translate new entries and build both locales.

## Contribution checklist

- Add or update tests for behavior changes.
- Update user documentation when commands, defaults, safety behavior, or
  supported platforms change.
- Update both English and Simplified Chinese pages in the same change.
- Keep examples executable and avoid documenting planned behavior as if it
  already exists.
- Run formatting, Clippy, workspace tests, type-checking, and the docs build.
