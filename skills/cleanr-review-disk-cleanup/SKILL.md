---
name: cleanr-review-disk-cleanup
description: "Review local disk-cleanup evidence with Cleanr and, only after the current user explicitly authorizes an exact reviewed plan, execute recoverable cleanup through the system trash. Use for `cleanr analyze`, recommendation states, age policy, plan review, authorized cleanup, manifests, or restore review. Keep paths local; never permanently delete files or empty the trash."
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

Before recommending cleanup:

1. Require a supported `schema_version`.
2. Require `scan.integrity = complete` for automatic preselection.
3. Read `policy.preselect_after_days`.
4. Explain each relevant `recommendation.state` and decision `codes`.
5. Exclude `suppressed` and `excluded` candidates.
6. Require human review for `review`, incomplete, conflicting, missing,
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

Do not edit the plan. Record the `sha256=` value printed by Cleanr. Inspect the
plan and require:

- the reviewed scan roots exactly match the approved scope;
- every selected item uses `planned_action = "trash"`;
- plan and item rollback methods are `system-trash+manifest`;
- selected items are deterministic `preselected` recommendations;
- no selected item is `review`, `suppressed`, or `excluded`.

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
