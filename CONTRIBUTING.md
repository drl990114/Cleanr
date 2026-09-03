# Contributing to Cleanr

Thanks for helping improve Cleanr. This guide covers development setup,
verification, documentation, and release work so the README can stay focused on
users.

## Prerequisites

- Rust 1.98.0 or a compatible newer toolchain.
- Node.js 20 or later for the documentation site.
- pnpm 10 for documentation dependencies.

## Repository Layout

```text
.github/        GitHub Actions workflows and release helpers
crates/         Rust workspace crates, grouped by responsibility
docs/           Docusaurus documentation site
npm/            npm launcher package and platform metadata
plugins/        Publishable plugin bundles and generated index metadata
scripts/        Local maintenance and release commands
```

## Build

Build the workspace:

```bash
cargo build
```

Build the release binary:

```bash
cargo build --release
```

The release binary is written to `target/release/cleanr`.

## Verify Changes

Run the same core checks as CI before opening a pull request:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo build --workspace --all-targets --all-features --locked
```

Validate generated JSON Schemas when plugin, language, configuration, or
manifest formats change:

```bash
cargo run --locked -p cleanr-cli -- plugin schema manifest >/dev/null
cargo run --locked -p cleanr-cli -- plugin schema index >/dev/null
cargo run --locked -p cleanr-cli -- plugin schema rules >/dev/null
cargo run --locked -p cleanr-cli -- plugin schema language >/dev/null
cargo run --locked -p cleanr-cli -- plugin schema config >/dev/null
```

## Documentation

Run the documentation site locally:

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

English source pages live in `docs/docs/`. Simplified Chinese pages live in
`docs/i18n/zh-Hans/docusaurus-plugin-content-docs/current/`.

After changing translated React text, navbar labels, footer labels, or sidebar
categories, regenerate translation keys:

```bash
pnpm docusaurus write-translations --locale zh-Hans
```

Then translate new entries and build both locales.

## README Localization

The root `README.md` is the primary English user-facing README. Localized README
copies live under:

```text
readme/en/
readme/zh-CN/
```

Keep README content concise and user-facing. Installation, safety behavior,
core features, and documentation links belong in README files. Development
setup, build commands, verification, release details, and repository internals
belong in this guide.

## Plugins and Rules

Built-in cleanup rules live in `crates/rules/builtin-plugins/` and are embedded
into release binaries. Publishable plugin bundles live under `plugins/`.

Validate a plugin bundle:

```bash
cleanr plugin validate plugins/<bundle-name>
```

Regenerate or check the static plugin index:

```bash
cleanr plugin index \
  --plugin-dir plugins \
  --base-url https://raw.githubusercontent.com/owner/repo/main/plugins

cleanr plugin index --check
```

The generated `plugins/index.json` contains SHA-256 metadata so the plugin
layout can be served from GitHub raw URLs, GitHub Pages, npm package CDNs, or
another static file host.

## npm Packages

The user-facing npm launcher package is `cleanr-cli`. Per-platform native
binary packages use the `@cleanr-cli/<os>-<cpu>` naming pattern and are
declared as optional dependencies of the launcher package.

## Release Process

Start from a clean worktree, then use the release script to synchronize Cargo
and npm versions, create the release commit and annotated tag, and push both to
`origin`:

```bash
./scripts/release.sh 0.2.0
```

To update and inspect version files without committing or pushing:

```bash
./scripts/release.sh 0.2.0 --prepare
```

Pushing a `vX.Y.Z` tag starts the release workflow. GitHub Actions checks the
tag version, formatting, Clippy, tests, and package contents before creating the
multi-platform GitHub Release and publishing workspace crates and npm packages.

For initial registry publishing, configure:

- `CARGO_REGISTRY_TOKEN`
- `NPM_TOKEN`

After the packages exist, prefer tokenless trusted publishing:

- Configure the `release.yml` GitHub workflow as a trusted publisher for every
  crate on crates.io, then set `CRATES_IO_TRUSTED_PUBLISHING=true`.
- Configure `release.yml` as the npm trusted publisher for the wrapper and all
  platform packages.
- Remove long-lived registry secrets after an OIDC release succeeds.

## Pull Request Checklist

- Add or update tests for behavior changes.
- Update user documentation when commands, defaults, safety behavior, or
  supported platforms change.
- Keep English and Simplified Chinese documentation in sync.
- Keep examples executable and avoid documenting planned behavior as if it
  already exists.
- Run formatting, Clippy, workspace tests, type-checking, and docs build when
  they are relevant to the change.
