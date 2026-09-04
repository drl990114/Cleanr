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

Run checks relevant to the changed crates before opening a pull request.
Formatting and linting use:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

Use targeted crate or named tests for behavioral changes, such as
`cargo test -p cleanr-core --locked <test-name>`. A normal local verification
pass does not require a build or the full workspace test suite. Required
release CI remains a separate gate.

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
```

English source pages live in `docs/docs/`. Simplified Chinese pages live in
`docs/i18n/zh-Hans/docusaurus-plugin-content-docs/current/`.

After changing translated React text, navbar labels, footer labels, or sidebar
categories, regenerate translation keys:

```bash
pnpm docusaurus write-translations --locale zh-Hans
```

Then translate new entries and inspect both locales in the development server.
The Pages workflow builds the deployable site separately.

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

Prepare a reviewable release before publishing. Record user-facing changes in
`CHANGELOG.md` under **Unreleased**; include format changes, upgrade limitations,
and known platform gaps. Category filtering and the new restore records are
not part of v0.14.0. See the [support matrix](docs/docs/support-matrix.md).

First inspect the worktree and preserve unrelated changes. For a chosen version
(replace `X.Y.Z` with the intended version):

```bash
./scripts/release.sh X.Y.Z --prepare
./scripts/release.sh X.Y.Z --check
```

Review the exact version changes and release notes. Confirm the required checks
for the exact release commit, including Windows, and the corresponding package
smoke checks. Local lint or a later passing commit is not validation of an old
published asset. Check the script's `--help` for its current publish gate.

Only after the release itself is authorized, run the publish command. The
script's default mode creates a release commit/tag and pushes them; do not use
it merely to preview a release:

```bash
./scripts/release.sh X.Y.Z --publish
```

After the workflow completes, verify the GitHub release, npm launcher/native
packages, and crates.io version externally. Run first-use checks against the
actual downloaded package, and label real Trash/restore tests separately from
source tests and read-only package smoke checks. Retain the evidence for the
supported platform matrix.

For registry publishing, use the existing workflow's trusted publishing setup
where available. Bootstrap tokens (`CARGO_REGISTRY_TOKEN`, `NPM_TOKEN`) should
be replaced by OIDC only after that path is configured and verified. Never
include credentials in release notes or reports.

## Documentation and public reports

The canonical documentation base is `https://drl990114.github.io/Cleanr/`, with
an uppercase `C`. After deployment, verify the homepage, CSS/JS, installation
pages, and English/Chinese links at the real URL. A type check does not prove
that Pages or its assets were deployed correctly.

Use generated or redacted paths in screenshots and walkthroughs. Record the
app version, OS, and scenario; label Unreleased features. Candidate/moved bytes
are not a measured increase in free space. Do not claim cloud-backed agents
keep tool output on the local machine or treat a caller-supplied approval flag
as an OS security boundary.

Use [support forms](SUPPORT.md) for public reports and [private security
reporting](SECURITY.md) for vulnerabilities. Avoid original analysis JSON,
plans, manifests, personal paths, and credentials in public attachments.

## Pull Request Checklist

- Add or update tests for behavior changes.
- Update user documentation when commands, defaults, safety behavior, or
  supported platforms change.
- Keep English and Simplified Chinese documentation in sync.
- Keep examples executable and avoid documenting planned behavior as if it
  already exists.
- Run relevant formatting, lint, targeted tests, documentation type-checking,
  and `git diff --check`; report real-runtime and deployment evidence separately.
