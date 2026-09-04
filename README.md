<div align="center">
  <h1>Cleanr</h1>
  <p><strong>Review developer caches. Choose what goes to Trash.</strong></p>
  <p>
    <a href="https://drl990114.github.io/Cleanr/">Documentation</a> ·
    <a href="https://github.com/drl990114/Cleanr/releases">Download</a> ·
    <a href="https://github.com/drl990114/Cleanr/discussions">Discussions</a>
  </p>
  <p>
    <a href="https://github.com/drl990114/Cleanr/actions/workflows/ci.yml"><img alt="CI workflow" src="https://img.shields.io/github/actions/workflow/status/drl990114/Cleanr/ci.yml?branch=main&label=CI&style=flat-square"></a>
    <a href="https://www.npmjs.com/package/cleanr-cli"><img alt="npm version" src="https://img.shields.io/npm/v/cleanr-cli?style=flat-square"></a>
    <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/github/license/drl990114/Cleanr?style=flat-square"></a>
  </p>
  <p><a href="readme/en/README.md">English</a> · <a href="readme/zh-CN/README.md">简体中文</a> · <a href="CONTRIBUTING.md">Contributing</a></p>
</div>

Cleanr helps developers review `node_modules`, Rust `target`, Xcode build output,
and package-manager caches. It explains each candidate in a keyboard-driven
terminal interface, validates your selection again, and moves selected items to
system Trash with a local restore record.

Start with one old project. Browser and application caches, logs, temporary
files, and downloads are additional, explicitly chosen scopes. Coverage varies
by platform; see the [support and verification matrix](https://drl990114.github.io/Cleanr/docs/support-matrix).

## First scan demo

![Cleanr v0.14.0 read-only scan of generated developer-cache samples](docs/static/img/cleanr-scan.png)

**v0.14.0 · macOS Apple Silicon.** Generated sample projects; read-only scan,
Review, and navigation. No cleanup or restore was performed.
[Watch the 34-second recording](https://drl990114.github.io/Cleanr/media/cleanr-first-scan.mp4)
· [Walkthrough details](https://drl990114.github.io/Cleanr/docs/demo/).

## Install

With Node.js 18 or later:

```bash
npm install --global cleanr-cli
cleanr --version
```

Or use `cargo install cleanr-cli` with Rust 1.98 or later, or download a native
binary from [GitHub Releases](https://github.com/drl990114/Cleanr/releases).
[Installation, updating, rollback, and removal](https://drl990114.github.io/Cleanr/docs/quick-start)
cover platform and architecture selection.

## Choose your first review

### In the terminal

```bash
cleanr /path/to/project
```

Replace the path with an existing project folder. Press `s` to scan, `r` to
review, `?` for help, and `q` to leave. This first walkthrough needs no cleanup.
Review normally shows candidates whose newest observed modification is at
least **90 days** old. An empty result can mean recent files, excluded paths,
or no matching rules; it does not mean the whole computer is clean.

After reviewing a candidate's reason and risk, use `space` to adjust selection
and `c` to open cleanup confirmation. `/restore` opens cleanup history.
See the [complete walkthrough](https://drl990114.github.io/Cleanr/docs/quick-start).

### With a coding agent

Install the optional cross-agent skill:

```bash
npx skills add drl990114/cleanr@cleanr-review-disk-cleanup -g
```

Ask: “Review this project's cleanup candidates with Cleanr. Explain the reasons
and risks before I choose anything.” The skill installs the CLI only when
missing. The underlying read-only entry point is:

```bash
cleanr analyze /path/to/project
```

Cleanr itself does not upload scan paths or reports. **An agent running on your
computer may send tool output to a cloud model.** Check that agent's data policy
before giving it access; use the TUI if you want to review without an AI service.
See [Evidence and privacy](https://drl990114.github.io/Cleanr/docs/evidence-and-privacy).

## What to expect

- Reasons, sizes, confidence, and risk notes for candidates; conservative
  selection based on trusted rules and observed modification age.
- Execution-time validation, overlapping-target checks, system Trash, and local
  cleanup and restore manifests.
- English and Simplified Chinese UI, declarative rule plugins, and native
  packages for macOS, Linux, and Windows.
- A versioned `analyze` JSON report and a separate, digest-checked `clean`
  command for plans the user has reviewed and explicitly authorized.

**Trash is recoverable storage, not immediate free space.** Moving files there
usually leaves their disk blocks allocated. Candidate and moved-byte totals are
not measured increases in free space. Emptying Trash is a separate user decision
and removes Cleanr's recovery path. Restore is best-effort and will not overwrite
an existing path.

The `--authorized-by-user` flag records the caller's assertion of approval;
Cleanr cannot independently verify who gave it. Digest and rescan checks protect
the reviewed plan through Cleanr's command path. They are not an OS sandbox for
an agent with other filesystem tools.

## Versions and help

Category filtering, cumulative filtered selection, and `Shift+A` global
selection apply to **0.15.0 and later**. Version 0.15.0 also introduces
`cleanr.restore.v2` restore records; read the compatibility notes before rolling
back. Check `cleanr --version` and the [changelog](CHANGELOG.md).

- [Safety and recovery](https://drl990114.github.io/Cleanr/docs/safety-and-recovery)
- [Troubleshooting](https://drl990114.github.io/Cleanr/docs/troubleshooting)
- [Support and feedback](SUPPORT.md) · [Security reporting](SECURITY.md)
- [Release readiness and verified platforms](https://drl990114.github.io/Cleanr/docs/support-matrix)

## Acknowledgements and license

Cleanr includes code adapted from [Byron/dua-cli](https://github.com/Byron/dua-cli),
an MIT-licensed disk usage analyzer by Sebastian Thiel and contributors.
Cleanr is licensed under the [MIT License](LICENSE).
