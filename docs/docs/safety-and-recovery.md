---
sidebar_position: 4
description: Learn what Cleanr protects, what cleanup changes, and when restore can or cannot succeed.
---

# Safety and recovery

Cleanr is designed to make cleanup reviewable and reversible, but "moved to
trash" is not the same as a permanent backup. This page defines the boundary.

## What happens during cleanup

For every selected item, Cleanr:

1. creates a pending execution manifest for the selected items;
2. verifies that the path is inside a scanned root;
3. rejects filesystem roots, symbolic links, and protected Cleanr paths;
4. checks that the item type, modification time, file size, and directory
   fingerprint still match the scan where applicable;
5. moves the item to the operating system trash;
6. records the result and restore locator back into the execution manifest.

Validation happens immediately before each item is moved. If a target changed
after the scan, that item fails safely and remains in place.

## Protected paths

Cleanr excludes:

- your home directory as a cleanup target;
- the active Cleanr executable and configuration file;
- Cleanr's state directory, including cleanup and restore history;
- configured plugin and language directories.

The contents below your home directory can still be scanned. The protection
prevents the home directory itself, or another protected subtree, from being
selected as one cleanup item.

## How selection works

The normal TUI review, `cleanr plan`, and `cleanr dry-run` first keep only
otherwise eligible candidates whose newest observed modification time across
the candidate tree is at least the effective threshold. The long-term setting
is `[recommendations].preselect_after_days` (90 days by default), and
`--inactive-days <DAYS>` overrides it for one invocation. `0` removes this age
filter and shows all otherwise eligible candidates.

An item is preselected only when all of the following are true:

- the rule confidence is `High`;
- the rule declares `default_selected = true`;
- the rule comes from Cleanr itself or an explicitly trusted plugin;
- its observed modification age satisfies the effective threshold;
- its scan evidence is complete and not future-dated.

Everything can still be deselected before cleanup. General downloads, logs,
broad temporary-file matches, medium-confidence items, and untrusted plugin
matches require manual selection. The narrow Windows rule for regular files
inside the current user's Temp directory is the exception: it requires the
file to be at least 30 days old and still passes the effective threshold. An
explicit `--select` may add an otherwise selectable review candidate with
recent or missing modification-time evidence to a plan. `cleanr analyze` and
the TUI `/usage` view retain complete evidence; `/usage` candidate and selected
metrics still apply the effective threshold.

Modification time is observed filesystem metadata, not proof of last access.
For a directory, Cleanr uses the newest observed modification time across the
candidate and its scanned descendants.

## Manifests and history

Cleanr stores:

- an execution manifest for every cleanup run;
- a restore manifest for every restore attempt;
- per-item success, failure, and rollback information.

These files live under the platform state directory in a `cleanr` folder.
They are required for Cleanr's restore history, so do not delete that directory
if you may need to undo a cleanup.

## Restore support

Programmatic restore is implemented for:

- macOS Trash;
- Windows Recycle Bin;
- Linux desktops with Freedesktop-compatible trash support.

Restore is best-effort. It cannot recover an item after the system trash has
been emptied, and it will not overwrite a path that has been recreated.
External tools that alter trash metadata can also make matching impossible.

If Cleanr cannot restore an item, inspect the system trash and the manifest
before taking further action.

## Confirmation and external local automation

`cleanr analyze` is read-only: it scans and prints evidence, but it does not
create a cleanup plan or move files. An agent recommendation or a preselected
item is not cleanup permission.

A local agent may execute cleanup only after the current user reviews and
explicitly authorizes an exact plan. The bounded command is:

```bash
cleanr clean \
  --plan cleanr-plan.json \
  --plan-sha256 <reviewed-sha256> \
  --authorized-by-user
```

`cleanr plan --output` prints the plan digest. Repeatable `--select` and
`--deselect` options can encode exact candidate-path choices the current user
made during evidence review; they cannot add unknown, overlap-suppressed, or
safety-excluded paths. Before execution, `clean` verifies the digest, re-scans
the recorded roots, rebuilds the plan with that exact selection, and compares
the selected execution projection with the authorized file. Any scope, rule,
recommendation policy, selection, safety, or selected-target filesystem drift
aborts cleanup and requires a new review and authorization. Candidate changes
outside the selected targets do not invalidate the plan. Successful items still
go only to system trash, with an execution manifest and restore locator. The
manifest records whether authorization came from local TUI confirmation or
explicit user delegation.

Setting `cleanup.require_confirm = false` removes the interactive confirmation
dialog for a direct local `/clean` request. It does not turn `analyze` into an
execution interface or remove the delegated command's digest and authorization
requirements.

Budget-limited scans never authorize cleanup. Their provenance is retained in analysis evidence,
and both plan creation and the execution layer fail closed before any trash or manifest operation.

## Practical safety checklist

- Start with a narrow project directory.
- Read the risk note for unfamiliar candidates.
- Keep source files and irreplaceable data in version control or backups.
- Do not empty the system trash until you are confident the cleanup was
  correct.
- Keep Cleanr's state directory while you still need restore history.
