---
description: Published versions, actual platform evidence, compatibility, and release preparation.
---

# Release readiness

This is a verification record and release checklist, not a claim that every
supported target has been tested on a user's machine. Last reviewed: 2026-09-04.

## Version scope and release verification

Category filtering, cumulative filtered selection, and `Shift+A` global
selection apply to **0.15.0 and later**. The compatibility notes below describe
the restore-record changes in 0.15.0. See the
[changelog](https://github.com/drl990114/Cleanr/blob/main/CHANGELOG.md) for details.

Consult the [v0.15.0 release](https://github.com/drl990114/Cleanr/releases/tag/v0.15.0)
for publication status and its per-platform `release-evidence.json`. The table
below records historical source checks and remaining Trash acceptance; it does
not replace installation checks on 0.15.0 assets. The [recorded walkthrough](./demo.md) retains its
v0.14.0 version label. A passing check on a later commit does not retroactively
validate an earlier release asset.

## Support and verification matrix

| Platform | Native distribution targets | Evidence available | Remaining acceptance |
| --- | --- | --- | --- |
| macOS | Apple Silicon, Intel | The source test `macos_system_trash_round_trip_records_exact_locator` passes in the macOS job of [CI for e136149](https://github.com/drl990114/Cleanr/actions/runs/33866868006). It moves and restores a test-owned directory with Chinese text and spaces, checking the exact Trash locator and nested file contents. | Repeat using the actual downloaded release package and supported OS/volume combinations. The source test is not proof for every filesystem or package. |
| Windows | x64, x86 | Recycle Bin cleanup/restore code and source CI exist. v0.14.0's source CI had two Windows TUI assertion failures; subsequent fixes are not part of that release. | Real Recycle Bin round trip and restored-content checks on Windows, plus downloaded-package first-run validation. |
| Linux | x86-64, ARM64, ARMv7 hard-float | Freedesktop Trash cleanup/restore code and source CI exist. | Real desktop Trash round trip and downloaded-package validation. Headless systems, mount policy, and Trash availability need explicit testing. |

Distribution targets are not a promise of identical cleanup coverage. The
Windows routine system scope is deliberately narrow; macOS has more named
application and developer-cache locations. A passed unit test, artifact creation,
or successful version command does not establish full cleanup/restore behavior.

## Updating and compatibility

Use the [installation guide](./quick-start.md#update-roll-back-or-uninstall) to keep
the install method consistent. Preserve local configuration and history before
an upgrade if you may need them. Do not assume an older binary can read a newer
plan or manifest. Never hand-edit a saved plan to bypass a compatibility error:
create and review a new plan with the version you intend to run.

**Compatibility change in 0.15.0:** new restore records use
`cleanr.restore.v2`; the 0.15.0 reader also accepts v1 history. `not-attempted` means
the backend was not called, while `pending` means the intent was saved but the
result was not recorded. After an interruption, pending items require manual
inspection of the original path, Trash, and manifest before retrying. Do not
roll back to an older binary to process v2 records; retain the state directory
and use the current or a newer compatible version.

In 0.15.0, cleanup and restore hold an OS file lock in the state directory for the
operation. Another operation reports a conflict; the OS releases the lock when
the process exits, so do not delete the lock file to bypass it. Trash and JSON
writes still cannot be one atomic transaction. If recording a filesystem result
fails, later items stop and the reported run/manifest details support manual
recovery.

Binary rollback does not undo file moves, restore history, or
configuration changes. Keep system Trash and manifests until recovery is no
longer needed.

## Before publishing

Use the repository's [release procedure](https://github.com/drl990114/Cleanr/blob/main/CONTRIBUTING.md#release-process).
Record the exact commit, package version, platform, and evidence for each gate:

- Required source checks and package smoke checks pass for the release commit.
- Each promoted installation path can run `--version`, `--help`, and an isolated
  read-only analysis. Test upgrade/removal separately; retain recovery records.
- The selected platform's real Trash/restore scenario passes with generated
  samples, including original-path conflicts and interrupted restoration.
- The deployed `/Cleanr/` website, CSS/JS, English and Chinese installation pages,
  and shared-card image respond successfully. A local fix alone is not deployment.
- Release notes distinguish published features, pending features, format changes,
  and known limitations. Verify the public GitHub, npm, and crates.io version
  after an explicitly authorized release.

A real Trash test changes the system Trash and needs its own execution scope;
read-only analysis or simulated backend tests do not authorize it.

## Before promoting

Lead with developer caches: one old project, reasons for each candidate, human
selection, and a recoverable move to Trash. Use a short real walkthrough with
synthetic or redacted paths, showing review and recovery as separate actions.
Identify the app version and platform; label repository-only visuals Unreleased.

Show candidate bytes and moved bytes accurately. Do not equate them with newly
available disk space or promise guaranteed recovery. Present the TUI and agent
entry points together, with the cloud-agent data boundary visible.

Start with a small developer audience. Track installation success, time to the
first understood review, empty-result confusion, unexpected rule matches, and
restore failures through voluntary feedback. Baselines and real-user demand are
**not yet measured** here; local scenario success cannot replace that evidence.
Use [support channels](https://github.com/drl990114/Cleanr/blob/main/SUPPORT.md)
and [private security reports](https://github.com/drl990114/Cleanr/security/advisories/new).
Do not request raw analysis JSON. Posting announcements or contacting users is
a separate publication decision.
