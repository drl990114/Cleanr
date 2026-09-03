---
sidebar_position: 2
description: Install Cleanr and safely complete your first scan, review, and cleanup.
---

# Quick start

## 1. Install Cleanr

Choose one installation method.

### npm

Requires Node.js 18 or later:

```bash
npm install --global cleanr-cli
```

The npm package installs a small launcher and the native binary for your
operating system and CPU.

### Cargo

Requires Rust 1.94 or later:

```bash
cargo install cleanr-cli
```

### Prebuilt binary

Download the binary for your platform from
[GitHub Releases](https://github.com/drl990114/cleanr/releases). On macOS or
Linux, make the downloaded file executable and place it somewhere on your
`PATH`.

### Build from source

```bash
git clone https://github.com/drl990114/cleanr.git
cd cleanr
cargo build --release
```

The binary is written to `target/release/cleanr`.

## 2. Launch the TUI

Run Cleanr in the directory you want to inspect:

```bash
cleanr
```

You can also choose one or more scan roots when launching:

```bash
cleanr ~/projects ~/Downloads
```

For a one-time candidate threshold without changing the configuration, add
`--inactive-days <DAYS>`:

```bash
cleanr --inactive-days 30 ~/projects
```

Cleanr opens on its home screen. It does **not** scan or clean anything until
you start an action.

## 3. Complete your first cleanup

Inside the TUI:

1. Press `s` to scan the configured roots.
2. When the scan finishes, press `r` to review candidates that meet the
   effective observed modification-age threshold.
3. Move with `j`/`k` or the arrow keys.
4. Press `space` to select or deselect an item.
5. Press `c` to review the selected total and open confirmation.
6. Choose **Yes**, then press `Enter` to move the selected items to trash.

Press `?` at any time to see the keyboard shortcuts. Press `Esc` or `x` during
a scan to cancel it.

:::tip Start conservatively

For a first run, scan one project rather than your whole home directory.
Review each candidate's reason and risk note before confirming.

:::

## Scan known system cleanup locations

Press `/`, type the following command, and press `Enter`:

```text
/scan --global
```

This searches known user-level cleanup locations for the current platform,
including developer caches, browser caches, app caches, temporary files, logs,
and Downloads. It does not mean "scan the entire disk."

To narrow the global scan, add one or more categories:

```text
/scan --global-kind browser-caches --global-kind logs
```

## Give a local AI tool read-only evidence

Use `analyze` when another local agent should inspect Cleanr's deterministic
facts rather than drive the TUI:

```bash
cleanr analyze ~/projects > cleanr-analysis.json
```

The command only scans and prints a versioned JSON report with complete
candidate evidence, including items outside the age threshold. It does not
create a cleanup plan or move files. The output contains real local paths, so
keep it on your machine unless you have independently removed sensitive
details. See [Evidence and privacy](./evidence-and-privacy) for the contract and
boundary. The long-term threshold is
`[recommendations].preselect_after_days` (90 days by default); use
`--inactive-days <DAYS>` for one invocation. Modification time is filesystem
metadata, not proof of last access.

Install the repository's cross-agent skill directly from GitHub:

```bash
npx skills add drl990114/cleanr@cleanr-review-disk-cleanup -g
```

It keeps review read-only by default. It can execute an exact plan only after
the current user reviews its summary and explicitly authorizes its SHA-256;
successful items go to system trash with a restore manifest. Agent selection
and invocation are covered in [Evidence and privacy](./evidence-and-privacy).

## Use Simplified Chinese

Initialize the built-in Chinese language file:

```bash
cleanr init --locale zh-CN
```

You can later open `/languages` in the TUI, select a language, and press
`Enter`. The selection is saved to the default configuration file.

## Next steps

- Learn all shortcuts and slash commands in [Using Cleanr](./using-cleanr).
- Understand the rollback boundary in
  [Safety and recovery](./safety-and-recovery).
- Exclude directories or change the theme in
  [Configuration](./configuration).
