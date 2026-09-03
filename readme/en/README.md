<div align="center">
  <h1>Cleanr</h1>
  <p><strong>Let your AI help you safely clean your disk with Cleanr.</strong></p>
  <p>
    <a href="https://drl990114.github.io/cleanr/">Documentation</a>
    ·
    <a href="https://github.com/drl990114/cleanr/releases">Download</a>
    ·
    <a href="https://github.com/drl990114/cleanr/discussions">Discussions</a>
  </p>
  <p>
    <a href="https://github.com/drl990114/cleanr/actions/workflows/ci.yml"><img alt="CI workflow" src="https://img.shields.io/github/actions/workflow/status/drl990114/cleanr/ci.yml?branch=main&label=CI&style=flat-square&logo=githubactions&logoColor=white"></a>
    <a href="https://github.com/drl990114/cleanr/actions/workflows/release.yml"><img alt="Release workflow" src="https://img.shields.io/github/actions/workflow/status/drl990114/cleanr/release.yml?label=release&style=flat-square&logo=githubactions&logoColor=white"></a>
    <a href="https://github.com/drl990114/cleanr/blob/main/LICENSE"><img alt="MIT License" src="https://img.shields.io/github/license/drl990114/cleanr?style=flat-square&color=0f766e"></a>
    <a href="https://www.npmjs.com/package/cleanr-cli"><img alt="npm version" src="https://img.shields.io/npm/v/cleanr-cli?style=flat-square&logo=npm"></a>
  </p>
  <p>
    <img alt="Rust" src="https://img.shields.io/badge/Rust-1.94-000000?style=flat-square&logo=rust&logoColor=white">
    <img alt="Ratatui" src="https://img.shields.io/badge/Ratatui-0.29-2563eb?style=flat-square">
    <img alt="Platforms: macOS, Linux, and Windows" src="https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-475569?style=flat-square">
    <img alt="Open source" src="https://img.shields.io/badge/open%20source-MIT-155eef?style=flat-square">
  </p>
  <p>
    <a href="../../README.md">Repository README</a>
    ·
    <a href="../zh-CN/README.md">简体中文</a>
    ·
    <a href="../../CONTRIBUTING.md">Contributing</a>
  </p>
</div>

Cleanr helps developers and macOS and Windows users find rebuildable generated
files and caches without turning cleanup into a blind delete. It scans paths
you choose, explains why each item matched, lets you review the plan in a
keyboard-driven terminal UI, and moves selected items to the operating system
trash.

## AI-Friendly by Design

Cleanr gives local coding agents deterministic, versioned JSON evidence through
`cleanr analyze` while keeping cleanup authority with the user. Agents can
inspect recommendation states, decision codes, risk notes, and scan integrity
without parsing terminal output. After the user reviews and explicitly
authorizes an exact plan, an agent can move its validated items to system trash
with a digest-bound command and local restore manifest. Raw paths and reports
stay local unless you explicitly choose to share them.

Install the cross-agent `cleanr-review-disk-cleanup` skill directly from GitHub:

```bash
npx skills add drl990114/cleanr@cleanr-review-disk-cleanup -g
```

The skill checks whether the Cleanr CLI is available, installs `cleanr-cli`
globally when needed, and guides local analysis plus explicitly authorized,
recoverable cleanup. See
[Evidence and privacy](../../docs/docs/evidence-and-privacy.md) for supported
agents, the report contract, and privacy guidance.

## Features

- Keyboard-driven scan, review, cleanup, and restore workflow.
- Built-in rules for common developer caches, browser caches, application
  caches, build output, package-manager caches, large downloads, logs, and
  temporary files. macOS coverage includes Brave and Arc, named cache-only
  locations for popular desktop apps, Homebrew, Xcode, CocoaPods, SwiftPM,
  diagnostic reports, and downloaded installers. Conservative Windows coverage
  adds only stale regular files from the current user's Temp and DirectX
  shader-cache directories.
- Reviewable cleanup plans with size, confidence, reason, and risk notes for
  every candidate.
- A local-only `cleanr analyze` JSON contract and a digest-bound
  `cleanr clean` command for exact plans explicitly authorized by the user.
- Conservative default selection: only high-confidence items from built-in or
  trusted rules can be preselected.
- Safer execution through trash-based cleanup, final pre-clean validation,
  overlap removal, and local cleanup manifests.
- Restore history for macOS Trash, Windows Recycle Bin, and
  Freedesktop-compatible Linux trash implementations.
- Declarative plugin support for custom cleanup rules and translations.
- Native packages for macOS, Linux, and Windows, with npm, Cargo, and GitHub
  Release installation options.
- English and Simplified Chinese UI support.

## Install

Install with npm:

```bash
npm install --global cleanr-cli
```

Install with Cargo:

```bash
cargo install cleanr-cli
```

You can also download a prebuilt binary from
[GitHub Releases](https://github.com/drl990114/cleanr/releases).

## Start

Run Cleanr in the directory you want to inspect:

```bash
cleanr
```

Or pass one or more scan roots:

```bash
cleanr ~/projects ~/Downloads
```

Inside the TUI, press `s` to scan, `r` to review candidates, `space` to select
or deselect an item, and `c` to confirm cleanup. Use `/scan --global` to inspect
known system cleanup locations and `/restore` to restore a previous cleanup run
when the platform supports it.

Press `?` in the TUI for keyboard help.

On macOS, inspect the routine user-level locations without changing anything:

```bash
cleanr analyze --global \
  --global-kind browser-caches \
  --global-kind app-caches \
  --global-kind logs \
  --global-kind temp-files \
  --global-kind downloads
```

Add `--global-kind developer-caches` when package-manager and Xcode caches are
also in scope. Trash contents, Mail data, iOS backups, Time Machine snapshots,
browser service workers, and system-owned roots are deliberately excluded.

On Windows, the conservative routine scope is intentionally smaller:

```bash
cleanr analyze --global \
  --global-kind app-caches \
  --global-kind temp-files
```

It matches only individual regular files that have not been modified for at
least 30 days in the current user's Temp or DirectX `D3DSCache` directory.
DirectX shader files are generated graphics caches that Windows can recreate.
The Temp and cache directories themselves are never selected. Explorer
thumbnail databases, crash dumps, Windows Update and Delivery Optimization
data, Prefetch, the Recycle Bin, registry data, Downloads, and system-owned
roots are deliberately excluded. Add browser or developer caches only when the
user explicitly includes that separate scope.

For a local coding agent, start with read-only analysis and keep its JSON on the
machine unless you deliberately redact it first:

```bash
cleanr analyze ~/projects > cleanr-analysis.json
```

The report is evidence for review, not a cleanup instruction. If the user wants
to delegate execution, first write and review an exact plan:

```bash
cleanr plan --output cleanr-plan.json ~/projects
cleanr clean --plan cleanr-plan.json \
  --plan-sha256 <reviewed-sha256> \
  --authorized-by-user
```

By default, the TUI review, `plan`, and `dry-run` keep only candidates whose
newest observed modification across the candidate tree occurred at least the
effective threshold ago. `plan` and `dry-run` also accept repeatable
`--select <exact-candidate-path>` and `--deselect <exact-candidate-path>`
options. An explicit `--select` can include an otherwise selectable review
candidate with recent or missing modification-time evidence; unknown,
suppressed, and safety-excluded paths are rejected.

`plan` prints the file's SHA-256. `clean` requires explicit authorization,
verifies that digest, re-scans, and rejects drift in the selected targets,
scan provenance, or safety policy. Changes limited to unselected candidates do
not invalidate the reviewed actions. Validated items then move to system trash
with a restore manifest; Cleanr never permanently deletes them.

The long-term setting is `[recommendations].preselect_after_days` in
`cleanr.toml`, with a default of 90 days. `--inactive-days <DAYS>` overrides it
for one invocation without changing the file; `0` removes the age filter and
shows all otherwise eligible candidates. `analyze` and the TUI `/usage` view
retain complete evidence; `/usage` candidate and selected metrics still use the
effective threshold. Modification time is filesystem metadata, not proof of
last access.

## Safety Model

Cleanr does not clean anything just because it was found. The plan remains
editable before authorization. Selected paths are validated again immediately
before cleanup, and items are moved to the operating system trash rather than
permanently deleted. Changing an authorized plan requires a new review and
authorization.

Restore is best-effort and depends on the system trash. Do not empty the trash
until you are confident the cleanup was correct.

## Learn More

- [Quick start](../../docs/docs/quick-start.md)
- [Using Cleanr](../../docs/docs/using-cleanr.md)
- [Safety and recovery](../../docs/docs/safety-and-recovery.md)
- [Configuration](../../docs/docs/configuration.md)
- [Plugins](../../docs/docs/plugins.md)

## Contributing

Development setup, checks, documentation workflow, and release notes live in
[CONTRIBUTING.md](../../CONTRIBUTING.md).

## Acknowledgements

Cleanr includes code adapted from
[Byron/dua-cli](https://github.com/Byron/dua-cli), an MIT-licensed disk usage
analyzer by Sebastian Thiel and contributors.

## License

Cleanr is licensed under the [MIT License](../../LICENSE).
