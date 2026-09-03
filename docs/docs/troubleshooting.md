---
sidebar_position: 8
description: Fix common Cleanr installation, terminal, scan, configuration, and restore problems.
---

# Troubleshooting

## Cleanr opens, but nothing is scanned

This is expected. Startup only sets the scan roots. Press `s`, run `/scan`, or
press `u` for a usage scan.

## The command palette does not show `/review` or `/clean`

Commands that require scan results stay hidden until a scan finishes. Run
`/scan` first. If a scan is still active, wait for it to finish or cancel it
with `Esc` or `x`.

## A scan finds no candidates

Check the following:

- The relevant pack is listed in `cleanup.enabled_rule_packs`.
- The target is not excluded by `ignore_dirs` or `ignore_patterns`.
- The item meets the rule's size, age, name, or path requirements.
- The item meets the effective `[recommendations].preselect_after_days`
  modification-age threshold. `cleanr analyze` retains below-threshold evidence;
  if that confirms a rule match, adjust the setting or rerun with an intentional
  `/scan --inactive-days <DAYS>` override.
- You scanned the directory that contains the candidate, not the candidate
  directory as the root. Scan roots themselves are never cleanup candidates.

Use `/rules` to inspect loaded rules and `/plugins` to confirm custom bundles
were discovered.

## `/scan --global` says no cleanup locations were found

The current platform did not report any known user-level system cleanup
locations for the selected global categories. You can still provide paths
explicitly:

```text
/scan /home/me/.cargo /home/me/.npm
```

Use absolute paths that exist on your operating system. Paths entered in the
TUI do not expand `~` or environment variables.

## Cleanr reports a configuration parse error

Print the active default path:

```bash
cleanr config path
```

If you use a custom file, repeat the same `--config` option when diagnosing.
Look for unknown keys, misspelled enum values, duplicate IDs, or invalid TOML.

To compare with a fresh default without overwriting your file:

```bash
cleanr --config /tmp/cleanr-default.toml config init
```

## The terminal display is unreadable

- Make sure the terminal supports Unicode and color.
- Cleanr uses portable ANSI colors by default. If colors still appear as solid
  red/green blocks, reset the terminal profile or try another terminal app.
- If the background brightness is wrong, set an explicit theme:

  ```bash
  cleanr config set ui.theme dark
  ```

- Resize very small terminal windows.
- If Cleanr is interrupted, run `reset` in the shell to restore terminal state.

## Cleanup skips an item that was selected

Cleanr revalidates every target immediately before execution. The item is
skipped if it changed after scanning, became a symbolic link, moved outside a
scan root, or overlaps a protected path. Rescan and review the new state rather
than forcing the old plan.

## Restore fails

Common causes are:

- the system trash was emptied;
- the item was manually removed from trash;
- the original path now exists;
- trash metadata changed or is unavailable;
- programmatic restore is unsupported by the platform.

Cleanr does not overwrite the current path. Inspect the system trash manually
and preserve Cleanr's state directory and manifests while investigating.

## Disable the update check

Cleanr checks for a new release at most once every 24 hours. Disable the
non-blocking startup check with:

```bash
cleanr --no-update-check
```

or:

```bash
export CLEANR_NO_UPDATE_CHECK=true
```

## A global scan is slow or appears stuck

Opt into the scan budgets described in [Configuration](./configuration) to bound retained entries,
elapsed time, estimated allocation, or retained diagnostics. Enabled budgets use one traversal
worker and return read-only partial evidence when reached. An elapsed limit is cooperative: it is
checked between scan phases and filesystem operations, but cannot interrupt a metadata or directory
read already blocked in the operating system kernel. Cancel the scan if the kernel call eventually
returns; investigate the filesystem, mount, or network share if it repeatedly blocks.

## Get more help

If the problem is reproducible, open an issue on
[GitHub](https://github.com/drl990114/cleanr/issues) with:

- your Cleanr version (`cleanr --version`);
- operating system and terminal;
- installation method;
- the exact command or key sequence;
- the complete error message with secrets and personal paths removed.
