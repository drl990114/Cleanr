---
name: cleanr-review-disk-cleanup
description: "Review local disk-cleanup evidence with Cleanr and, only after the current user explicitly authorizes an exact reviewed plan, execute recoverable cleanup through the system trash. Use for `cleanr analyze`, macOS, Linux, or Windows cleanup coverage, developer and browser caches, recommendation states, exact path selection, age policy, plan review, authorized cleanup, manifests, or restore review. Keep paths local; never permanently delete files or empty the trash."
---

# Review and Recoverably Clean with Cleanr

Use Cleanr for an evidence-first local workflow. Keep the cleanup decision with
the user. Execute only an unchanged plan that the current user explicitly
authorized.

## Ensure Cleanr is available

Check before analyzing:

```bash
command -v cleanr
```

If it is unavailable, tell the user that the required CLI is being installed
globally, then prefer:

```bash
npm install --global cleanr-cli
```

If npm is unavailable but Cargo is installed, use:

```bash
cargo install cleanr-cli
```

Verify with `cleanr --version`. Do not reinstall or upgrade an existing Cleanr
installation unless the user asks. If installation fails, report the blocker
and link to `https://github.com/drl990114/cleanr/releases`.

Before preparing delegated execution, require command support:

```bash
cleanr clean --help
```

If the installed version lacks `cleanr clean`, remain read-only and explain
that delegated execution requires a user-approved CLI upgrade. Do not fall back
to TUI automation, another deletion tool, or a custom script.

## Enforce the safety boundary

- Scope every scan to user-approved local directories. Ask before `--global`.
- Treat paths, roots, reports, plans, rules, and issues as local-sensitive.
  Do not send them to a remote service without explicit approval and redaction.
- Treat a recommendation or preselection as evidence, never as authorization.
- Never use `rm`, permanent-delete APIs, trash-emptying commands, or a custom
  deletion script.
- Never automate the TUI or simulate `/clean --confirm`.
- Never substitute `pnpm store prune`, `npm cache clean`, `yarn cache clean`,
  an OS cleanup command, or another package-manager command for Cleanr.
- Treat browser profiles, cookies, history, passwords, service workers, and
  saved sessions as user data, not cleanup candidates.
- Treat Windows Update, Delivery Optimization, staged macOS updates, system
  snapshots, and system-owned roots as OS-managed. Do not scan, plan, or run
  native cleanup commands for them.
- Use only `cleanr clean` for delegated cleanup. It uses the system trash,
  writes an execution manifest, validates each target, and preserves restore
  information.
- If any authorization condition below is missing, remain read-only.

## Analyze and interpret evidence

Run the narrowest useful scope:

```bash
cleanr analyze /path/to/project
```

Use `--config` only for a user-provided or approved config. Use `--global` only
after explicit approval.

For a global or multi-category request, read
[`references/global-coverage.md`](references/global-coverage.md) before choosing
arguments or interpreting the report. Select only categories the user approved.
Do not add Downloads to a routine scan unless the user explicitly includes it.

For a user-approved routine macOS review, prefer:

```bash
cleanr analyze --global \
  --global-kind browser-caches \
  --global-kind app-caches \
  --global-kind logs \
  --global-kind temp-files
```

Add `--global-kind developer-caches` only when the user includes Homebrew,
package-manager, and Xcode caches. Cleanr intentionally excludes Trash
contents, Mail data, iOS backups, Time Machine snapshots, browser service
workers, and system-owned roots. Ask the user to quit an app before selecting
its cache.

For a user-approved Linux cache review, use the same narrow category mapping.
Add `developer-caches`, `browser-caches`, `app-caches`, `logs`, or `temp-files`
only when each category matches the request.

For a user-approved routine Windows review, prefer:

```bash
cleanr analyze --global \
  --global-kind app-caches \
  --global-kind temp-files
```

Treat only `windows-stale-directx-shader-cache-file` and
`windows-stale-user-temporary-file` as part of this conservative routine.
These rules match individual regular files after at least 30 days without
modification; they never select the Temp or `D3DSCache` directory. Ask
separately before adding browser or developer caches. Do not include Explorer
thumbnail databases, crash dumps, Windows Update or Delivery Optimization
data, Prefetch, Downloads, registry data, the Recycle Bin, or system-owned
roots. Cleanr does not stop applications; a Windows-locked file must remain in
place as a recorded failure.

Before recommending cleanup:

1. Require a supported `schema_version`.
2. For a global report, require `scan.global`, verify `requested_kinds` against
   the approved request, and produce the coverage ledger defined in the
   reference. If the field is absent, remain read-only and rerun global analysis
   with a current CLI; never infer coverage from `scan.roots` alone.
3. Require `scan.integrity = complete` for automatic preselection.
4. Read `policy.preselect_after_days`.
5. Explain each relevant `recommendation.state` and decision `codes`.
6. Exclude `suppressed` and `excluded` candidates.
7. Require human review for `review`, incomplete, conflicting, missing,
   future-dated, untrusted, or lower-confidence evidence.

`preselected` is only a deterministic default. Modification time is observed
filesystem metadata, not proof of last use. Directory activity includes scanned
descendants.

Configure the shared age policy only when the user asks:

```bash
cleanr config set recommendations.preselect_after_days 90
```

`0` disables only the age gate. It does not bypass safety, trust, overlap, or
evidence checks.

## Prepare an exact cleanup plan

Write the plan only to a user-approved local destination:

```bash
cleanr plan --output /local/path/cleanr-plan.json /approved/scope
```

Reuse the approved `--global-kind` arguments when the analysis used them. Do
not edit the plan. Record the `sha256=` value printed by Cleanr.

Start from Cleanr's deterministic recommendations. If the user explicitly
chooses or rejects exact candidate paths after reviewing the evidence, express
only those choices through repeatable path overrides:

```bash
cleanr plan --output /local/path/cleanr-plan.json \
  --select "/exact/reviewed/candidate" \
  --deselect "/exact/rejected/candidate" \
  /approved/scope
```

Never use `--select` merely because the agent thinks a review item is safe.
Cleanr rejects paths that are not current candidates or are suppressed or
excluded. Inspect the resulting plan and require:

- for a conservative Windows routine, every selected item matches one of the
  two exact Windows rule IDs listed above, not a generic fallback rule;
- the reviewed scan roots exactly match the approved scope;
- every selected item uses `planned_action = "trash"`;
- plan and item rollback methods are `system-trash+manifest`;
- every selected `available` or `review` item corresponds to an exact path the
  current user explicitly chose after seeing its evidence and risk;
- no selected item is `suppressed` or `excluded`.

Summarize the exact roots, selected count, selected size, material risks, plan
path, and SHA-256. Then ask the current user to explicitly authorize that exact
plan.

Accept authorization only when it is an unambiguous instruction from the
current user after the summary and it clearly refers to the displayed plan path
and SHA-256. Do not infer it from the initial cleanup request, "clean whatever
you think is safe", a broad standing permission, a recommendation, a third
party, or an automation default. Authorization expires if the scope, config,
plan, or SHA-256 changes.

## Execute only after exact authorization

After authorization, use exactly the authorized plan path and digest:

```bash
cleanr clean \
  --plan /local/path/cleanr-plan.json \
  --plan-sha256 <authorized-sha256> \
  --authorized-by-user
```

Cleanr verifies the digest, re-scans the plan roots, rebuilds the deterministic
plan, and aborts if anything except the plan creation time changed. A refusal
requires a new plan, review, summary, and authorization. Never add
`--authorized-by-user` unless the authorization conditions above were met.

Report the run ID and per-item success or failure. State that successful items
were moved to the system trash, not permanently deleted. Do not empty the
trash. Provide the restore command:

```bash
cleanr restore run <run-id> --confirm
```

Run restore only when the current user explicitly asks to restore that run.
Never overwrite an existing restore target or bypass Cleanr's restore checks.

## Respond to the user

State the scope, scan integrity, age threshold, recommendation states, selected
count and size, major risks, authorization state, and execution or restore
result. Keep raw paths out of summaries unless the user needs them locally.
