---
sidebar_position: 3
description: Learn Cleanr's scan, review, cleanup, restore, keyboard, and slash-command workflow.
---

# Using Cleanr

:::note Version scope

Category labels, `f` filtering, cumulative filtered selection, and `Shift+A`
global selection below are **Unreleased** changes after v0.14.0. Check
`cleanr --version` and the [changelog](https://github.com/drl990114/Cleanr/blob/main/CHANGELOG.md).
For an older installed version, its `?` help is authoritative for shortcuts.

:::

## Choose what to scan

The paths passed at startup become the default scan roots:

```bash
cleanr ~/projects/app-one ~/projects/app-two
```

Use `--inactive-days <DAYS>` to override the configured candidate age for this
invocation without changing the configuration file:

```bash
cleanr --inactive-days 30 ~/projects/app-one
```

If no path is provided, the current directory is used. Starting Cleanr does not
immediately scan those paths; press `s` or run `/scan`.

You can replace the current roots from the command palette:

```text
/scan /home/me/projects/app-one /home/me/Downloads
```

Add `--global` to scan known system cleanup locations in addition to any paths
you provide:

```text
/scan /home/me/projects --global
```

From the command palette, press `/`, type `global`, and press `Enter` to select
the `/scan --global` shortcut without remembering the flag.

Use `--global-kind` to narrow the global preset. Passing a kind automatically
enables global scanning:

```text
/scan --global-kind browser-caches
```

Override the configured modification-age threshold for one scan with:

```text
/scan --inactive-days 30
```

For a routine Windows review, prefer the narrower file-only scope:

```text
/scan --global-kind app-caches --global-kind temp-files
```

On Windows, that scope discovers only the current user's DirectX `D3DSCache`
and user Temp directory. Its high-confidence Windows rules match regular files,
never either directory, after at least 30 days without modification. Add
`browser-caches` or `developer-caches` only when the user wants those separate
scopes.

Paths typed inside the TUI are not expanded by a shell, so `~` and environment
variables remain literal text. Use absolute paths. For paths containing spaces,
pass the quoted path when launching Cleanr instead.

## Review and select candidates

After a scan, press `r` or run `/review`. Each candidate row shows its size,
category, and path; details show the full category, matched rules, confidence,
reason, and risk note. By default, the view includes only
candidates whose newest observed modification time across the candidate tree
meets the configured threshold, which is 90 days by default.

High-confidence items from built-in or trusted rules can be preselected.
Medium- and low-confidence items, and all matches from untrusted plugins, start
unselected.

Change the long-term threshold with
`[recommendations].preselect_after_days`, or use `--inactive-days <DAYS>` for
one invocation. `0` removes the age filter and shows all otherwise eligible
candidates. Modification time is filesystem metadata, not proof of last access.

Categories describe rule content, such as build caches or logs; they are
separate from the locations chosen with `--global-kind`. Built-in categories
use translated labels, while custom plugin categories keep their original
names. Conflicting rules with different effective categories appear under
**Multiple categories**, with details listing the categories and conflict.

Press `f` to open a single-category filter with each category's candidate count
and size. Choose with `↑` / `↓` or `j` / `k`, then press `Enter` to apply or
`Esc` to cancel. Filtering preserves selections across categories. The list
shows the filtered count and the global selection total, including the count
and size selected outside the filter. Switching views keeps the filter;
starting a new scan, including the automatic scan after cleanup, resets it
to **All**. Partial results without a cleanup plan show tentative categories
and remain read-only.

Useful keys while reviewing:

| Key | Action |
| --- | --- |
| `j` / `k`, `↓` / `↑` | Move through the list |
| `gg` / `G` | Jump to the first / last item |
| `Ctrl+f` / `Ctrl+b` | Page down / up |
| `space` or `Enter` | Select or deselect the current item |
| `f` | Open the category filter |
| `a` or `%` | Select all items in the current filter, across all pages; deselect them if all are selected |
| `Shift+A` | Select all candidates globally; deselect them if all are selected |
| `c` | Confirm cleanup of all selected items, including those outside the filter |
| `h` or `Esc` | Return home |
| `?` | Open keyboard help |
| `q` | Quit |

Numeric prefixes work with list movement. For example, `5j` moves down five
items and `12G` jumps to item 12.

## Clean selected items

Press `c` or run `/clean` to review the selected count and size. With the
default configuration, Cleanr asks for confirmation and initially selects
**No**. Cleanup uses the global selection. When a category filter hides selected
items, the confirmation also states their count and size.

After confirmation, each selected item is validated again and moved to the
system trash. Failures are recorded per item; one failed item does not hide
the result of the others.

`/clean --confirm` skips the confirmation dialog and executes the current
selection as an explicit local user action. Use it only after reviewing the
plan.

## Restore a cleanup run

Run `/restore`, select a cleanup run, and press `Enter`. Confirm the restore to
move available items back to their original paths.

Restore can fail when:

- an item has already been removed from the system trash;
- another file or directory now exists at the original path;
- the operating system cannot identify the original trash item;
- the platform does not support programmatic restore.

Cleanr never overwrites an existing restore target.

## Non-interactive commands

Use these commands from scripts or terminals when you do not need the TUI:

```bash
cleanr scan --json /path/to/project
cleanr analyze /path/to/project
cleanr plan --output cleanr-plan.json /path/to/project
cleanr --inactive-days 30 plan --output cleanr-plan.json /path/to/project
cleanr plan --output cleanr-plan.json --select /exact/candidate /path/to/project
cleanr dry-run --json /path/to/project
cleanr clean --plan cleanr-plan.json --plan-sha256 <reviewed-sha256> --authorized-by-user
cleanr restore list
cleanr restore run <run-id> --confirm
```

`analyze` always prints a versioned, local `AnalysisReport` JSON document with
the complete candidate evidence, including items outside the age threshold. It
does not create a cleanup plan or move files. Its output contains real local
paths, so use it only with a local agent unless you independently redact the
data. `dry-run` and `plan` only generate a cleanup plan.

The human-readable `cleanr scan` candidate count uses the effective age
threshold. `cleanr scan --json` keeps the raw scan entries.

`plan` and `dry-run` normally keep only candidates that satisfy the effective
modification-age threshold. Repeat `--select <path>` or `--deselect <path>` to
encode exact choices made during evidence review. An explicit `--select` can
include an otherwise selectable review candidate with recent or missing
modification-time evidence. A selected path must exist, match a candidate from
that scan, and not be overlap-suppressed or safety-excluded. An agent must not
choose a review-only candidate without an explicit candidate-path decision from
the current user. Do not edit the generated plan.

When `plan` writes a file, it prints that file's SHA-256. `clean` is intended
for an exact plan that the current user has already reviewed and explicitly
authorized. It verifies the supplied digest, re-scans the plan roots, rebuilds
the plan with the exact reviewed selection, and refuses execution if any
selected target, scan provenance, or safety policy changed. Changes limited to
unselected candidates do not invalidate the reviewed actions. It only moves
validated items to the system trash and records an execution manifest; it never
permanently deletes them. Restore still requires `--confirm`.

## Slash commands

Press `/` to open the command palette. Commands that need scan results appear
after a scan finishes.

| Command | What it does |
| --- | --- |
| `/scan [path...] [--global] [--global-kind=<kind>] [--inactive-days=<days>]` | Scan paths or known system cleanup locations with an optional one-scan age override |
| `/scan --global` | Scan all known system cleanup locations |
| `/usage [path...] [--global] [--global-kind=<kind>] [--inactive-days=<days>]` | Scan and open the disk-usage summary with an optional recommendation-metric age override |
| `/usage --global` | Scan known system cleanup locations and open usage |
| `/review` | Build and show cleanup candidates |
| `/plan` | Build the current cleanup plan |
| `/clean` | Review the current selection and request confirmation |
| `/clean --confirm` | Execute the current selection without the dialog |
| `/export-plan [path]` | Write the plan as JSON; defaults to `cleanr-plan.json` |
| `/restore` | Open cleanup history and restore a run |
| `/rules` | Show active rule packs and rules |
| `/plugins` | Show loaded declarative plugins |
| `/languages` | Show and switch installed languages |
| `/tasks` | Show task activity from the current session |
| `/help` | Open keyboard help |
| `/quit` | Quit Cleanr |

`/stats` is an alias for `/usage`, `/lang` for `/languages`, and `/q` for
`/quit`.

## Inspect disk usage without cleaning

Press `u` or run `/usage`. This performs a scan and opens a size-oriented view.
It keeps the complete usage entries. Its candidate and selected summary metrics
apply the effective age threshold; `/usage --inactive-days <DAYS>` overrides
that threshold for this scan.
It does not move files or automatically execute a cleanup plan.

## Cancel or leave safely

- During a scan, press `Esc` or `x` to request cancellation.
- Outside a scan, `Esc` or `h` returns home.
- `q` or `Ctrl+C` exits Cleanr and restores the terminal.
