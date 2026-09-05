<div align="center">
  <h1>Cleanr</h1>
  <p><strong>AI-friendly disk cleanup. Safety comes first.</strong></p>
  <p>
    <a href="https://drl990114.github.io/Cleanr/">Documentation</a> ·
    <a href="https://github.com/drl990114/Cleanr/releases">Download</a> ·
    <a href="https://github.com/drl990114/Cleanr/discussions">Discussions</a>
  </p>
  <p>
    <a href="https://github.com/drl990114/Cleanr/actions/workflows/ci.yml"><img alt="CI workflow" src="https://img.shields.io/github/actions/workflow/status/drl990114/Cleanr/ci.yml?branch=main&label=CI&style=flat-square"></a>
    <a href="https://www.npmjs.com/package/cleanr-cli"><img alt="npm version" src="https://img.shields.io/npm/v/cleanr-cli?style=flat-square"></a>
    <a href="../../LICENSE"><img alt="MIT License" src="https://img.shields.io/github/license/drl990114/Cleanr?style=flat-square"></a>
  </p>
  <p><a href="../../README.md">English</a> · <a href="../zh-CN/README.md">简体中文</a> · <a href="../../CONTRIBUTING.md">Contributing</a></p>
</div>

Cleanr is an AI-friendly, cross-platform disk cleanup tool. Its structured,
read-only analysis lets an AI agent explain cleanup candidates and help you
prepare a plan. You can also review candidates directly in the terminal.

It covers application and browser caches, logs, temporary files, downloads, and
development artifacts such as `node_modules`, Rust `target`, and package-manager
caches. Each candidate comes with reasons and risk notes; you decide what moves
to system Trash, with a local restore record.

**Safety comes first.** Analysis is read-only, cleanup confirmation is on by
default, and selected paths and file state are checked again before each move.
Agent execution verifies the reviewed plan against a fresh scan. Items go to
system Trash with local records for best-effort restore.
[Safety checks and recovery limits](https://drl990114.github.io/Cleanr/docs/safety-and-recovery).

Choose the folders or known cleanup locations you want to review. Coverage varies
by platform; see [scanning options](https://drl990114.github.io/Cleanr/docs/using-cleanr)
and the [support and verification matrix](https://drl990114.github.io/Cleanr/docs/support-matrix).

## First scan demo

![Cleanr v0.14.0 read-only scan of generated developer-cache samples](../../docs/static/img/cleanr-scan.png)

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

### With an AI agent

Install the optional cross-agent skill:

```bash
npx skills add drl990114/cleanr@cleanr-review-disk-cleanup -g
```

Ask: “Use Cleanr to review app caches and temporary files. Summarize the
candidates, reasons, and risks, then wait for my selection.” The skill installs
the CLI only when missing. The underlying read-only entry point is:

```bash
cleanr analyze --global \
  --global-kind app-caches \
  --global-kind temp-files
```

Cleanr itself does not upload scan paths or reports. **An agent running on your
computer may send tool output to a cloud model.** Check that agent's data policy
before giving it access; use the TUI if you want to review without an AI service.
See [Evidence and privacy](https://drl990114.github.io/Cleanr/docs/evidence-and-privacy).

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

## Acknowledgements and license

Cleanr includes code adapted from [Byron/dua-cli](https://github.com/Byron/dua-cli),
an MIT-licensed disk usage analyzer by Sebastian Thiel and contributors.
Cleanr is licensed under the [MIT License](../../LICENSE).
