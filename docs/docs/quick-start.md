---
sidebar_position: 2
description: Install Cleanr and complete a first read-only review of a project.
---

# Quick start

Start with one project you recognize. The first goal is to see and understand
candidates; no cleanup is required.

## 1. Install and verify

Choose one method:

| Method | Requirement | Install |
| --- | --- | --- |
| npm | Node.js 18 or later | `npm install --global cleanr-cli` |
| Cargo | Rust 1.98 or later | `cargo install cleanr-cli` |
| Native binary | A matching OS and CPU | Download from [GitHub Releases](https://github.com/drl990114/Cleanr/releases) |

Then run:

```bash
cleanr --version
cleanr --help
```

For manual downloads, match the target name to your machine:

| Platform | Release target |
| --- | --- |
| macOS Apple Silicon | `aarch64-apple-darwin` |
| macOS Intel | `x86_64-apple-darwin` |
| Linux x86-64 | `x86_64-unknown-linux-musl` |
| Linux ARM64 | `aarch64-unknown-linux-musl` |
| Linux ARMv7 hard-float | `arm-unknown-linux-gnueabihf` |
| Windows x64 | `x86_64-pc-windows-msvc` |
| Windows x86 | `i686-pc-windows-msvc` |

Use `uname -m` on macOS/Linux or **Settings → System → About → System type**
on Windows to check the architecture. A native Windows ARM64 package is not
listed. Availability and actual verification are separate; see the
[verification matrix](./support-matrix.md).

Extract the archive if the release asset is archived. On macOS/Linux, make the
`cleanr` file executable with `chmod +x /path/to/cleanr`, then place it in a
user-owned directory on `PATH`, such as `~/.local/bin`. Add that directory to
your shell's `PATH` if needed. On Windows, place `cleanr.exe` in a user-owned
folder on your user `Path`, then open a new terminal. In PowerShell, test a
file in the current directory with `.\cleanr.exe --version`.

If your OS blocks a downloaded binary, confirm the source and release asset
before proceeding. Cleanr does not promise notarization or a signing status
unless that release documents it; do not disable your OS's security protections.

## 2. Review without cleaning

Launch with a real project directory, quoting paths that contain spaces:

```bash
cleanr "/path/to/project"
```

For example, use `cleanr "$HOME/projects/my-app"` in a POSIX shell or
`cleanr "$HOME\projects\my-app"` in PowerShell, if that directory exists.
Startup sets the root; it does not scan automatically.

1. Press `s` to scan the project.
2. When the scan finishes, press `r` to review candidates.
3. Move with the arrow keys or `j` / `k`, and read the reason and risk note.
4. Press `?` for help or `q` to leave.

Success means the scan completes and you understand the results, even when
there are no candidates. `Esc` or `x` cancels an active scan.

**Why might the list be empty?** Review normally shows only matching candidates
whose newest observed modification across the candidate tree is at least
**90 days** old. Recent projects, excluded paths, and unmatched folders can
produce an empty list. The scan root itself is never a cleanup candidate: scan
the project containing `target` or `node_modules`, not that generated directory
as the root. This does not establish that the whole computer is clean.

To inspect complete rule evidence without moving files:

```bash
cleanr analyze "/path/to/project"
```

`analyze` includes below-threshold candidates. If you intentionally want a
different review threshold, use `cleanr --inactive-days 30 "/path/to/project"`.
`0` removes the age filter; it does not remove safety checks. The persistent
setting is `[recommendations].preselect_after_days`. Modification time is not
proof of last use.

## 3. Choose an optional next step

### Review a cleanup

After reading the candidates, press `space` to adjust selection and `c` to
review the selected total and open confirmation. With the default configuration,
choose **Yes** and press `Enter` only when you want those items moved to Trash.

Trash usually keeps the file's disk blocks allocated. The displayed candidate
or moved-byte total is not measured free space. `/restore` opens cleanup
history; recovery needs the Trash item and manifest and cannot overwrite an
existing path. See [Safety and recovery](./safety-and-recovery.md) before cleanup.

### Review with a coding agent

Install the optional cross-agent skill:

```bash
npx skills add drl990114/cleanr@cleanr-review-disk-cleanup -g
```

Ask the agent to explain one project's candidates before you choose any action.
The skill installs a missing CLI, but does not upgrade an existing one.
`cleanr analyze` is read-only. Cleanr does not upload its report; an agent may
send tool output to a cloud model even when its tools run on your computer.
Check [Evidence and privacy](./evidence-and-privacy.md) first.

If saving a report, put it outside the scan roots. Shell redirection creates or
truncates its output file; it is a separate write from Cleanr's read-only scan.
Do not post the original JSON to an issue or an external service.

### Scan known cache locations

Use the TUI command palette (`/`) and enter a narrow scope:

```text
/scan --global-kind developer-caches
```

`--global` means known user-level locations, not the whole disk. Select browser,
application, log, temporary-file, or download categories only when you want to
review them. Routine Windows `app-caches` and `temp-files` coverage is limited
to stale regular files in user Temp and DirectX `D3DSCache`; it is not a Windows
system optimizer. See [Using Cleanr](./using-cleanr.md).

## Update, roll back, or uninstall

Use the same installation method to avoid duplicate executables. Confirm which
one runs with `command -v cleanr` on macOS/Linux or `Get-Command cleanr` in
PowerShell. Check the release notes before changing versions.

| Method | Update | Uninstall |
| --- | --- | --- |
| npm | `npm install --global cleanr-cli@latest` | `npm uninstall --global cleanr-cli` |
| Cargo | `cargo install cleanr-cli --locked --force` | `cargo uninstall cleanr-cli` |
| Native binary | Replace the executable with the matching new release asset | Remove only that executable and any PATH entry you added |

To install a specific previously published version, replace `X.Y.Z` with its
version number and use `npm install --global cleanr-cli@X.Y.Z` or
`cargo install cleanr-cli --version X.Y.Z --locked --force`. For a manual
installation, download that version's matching asset. A binary rollback does
not guarantee older versions can read newer configuration, plans, or manifests;
consult the [compatibility notes](./support-matrix.md).

Before upgrading, keep a local copy of configuration and cleanup/restore state
if you need it. `cleanr config path` shows the default configuration path and
`cleanr restore list` lists runs. Uninstalling the executable is not a request
to remove those records or empty Trash. Retain both while recovery may be needed.

## Language and help

Use `cleanr init --locale zh-CN` to initialize the Chinese language file, or
select an installed language in `/languages`. Initialization may write
configuration or language files; it is separate from the read-only walkthrough.

[Using Cleanr](./using-cleanr.md) covers shortcuts;
[Troubleshooting](./troubleshooting.md) covers installation and empty results.
Category filtering, cumulative filtered selection, and `Shift+A` global
selection require **0.15.0 or later**. That version also introduces
`cleanr.restore.v2` records; review the [compatibility notes](./support-matrix.md)
before changing versions.
