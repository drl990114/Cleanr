# Authorized execution

Read this file only when the user wants cleanup planning, plan execution, or a
restore. Complete each section in order. Keep report and plan files local;
read path-bearing tool output only within the report-processing scope agreed
in Stage 1 of the skill. Local files do not imply on-device model processing.

## Prepare one exact plan

Write the plan outside the scanned roots. Use the user's destination when one
was provided; otherwise create a fresh private OS temporary directory, state
that choice, and never overwrite an existing file:

```bash
cleanr plan --output /local/path/cleanr-plan.json /approved/scope
```

Reuse the approved `--config` and `--global-kind` arguments. If analysis used
`--inactive-days <DAYS>`, pass the same override to `plan`; do not substitute
the current configuration value. Do not edit the plan. Record the `sha256=`
value printed by Cleanr.

Start from Cleanr's deterministic recommendations. For a broad cleanup request,
do not add path overrides. Use them only to encode exact candidate paths the
current user explicitly named as selection intent; the final summary must show
their evidence and risk before confirmation:

```bash
cleanr plan --output /local/path/cleanr-plan.json \
  --select "/exact/user-selected/candidate" \
  --deselect "/exact/user-rejected/candidate" \
  /approved/scope
```

Never use `--select` because the agent independently considers an item safe or
because it belongs to a generally requested category. Cleanr must reject paths
that are not current candidates or are suppressed or excluded.

Before requesting authorization, verify:

- the plan roots, config, global categories, and inactivity policy exactly
  match the reviewed analysis;
- every selected item is an exact reviewed candidate;
- every action is `trash`;
- plan and item rollback methods are `system-trash+manifest`;
- no selected item is `suppressed` or `excluded`;
- every selected `available` or `review` item was an exact path explicitly
  requested by the current user and is prominently identified for post-evidence
  confirmation;
- for the conservative Windows routine, every selected item uses
  `windows-stale-directx-shader-cache-file` or
  `windows-stale-user-temporary-file`, not a generic fallback rule.

If `selected_count` is zero, explain why and stop without an execution prompt.

## Translate the JSON into a human summary

Do not paste the plan JSON or require the user to inspect it. Build the summary
from selected `items` and their retained evidence, using the user's language.

Start with one total line: selected item count, selected total size,
file/directory mix, and `system-trash+manifest` rollback. Then group selected
items by the most specific human rule label from `evidence.matched_rules`;
fall back to `category` plus `rule_id` when no label is available.

For every group, show:

- what it is, in user language, and whether it contains files or directories;
- item count and summed `size_bytes`;
- why Cleanr considers it rebuildable or removable, deduplicating `reason`;
- the concrete consequence, deduplicating `risk_note`;
- confidence and recommendation state, highlighting any explicit-path
  `available` or `review` selection;
- up to three representative local paths. If a group has at most ten items,
  list every path instead.

Do not call directories files. Selected size is not a measured free-space
increase; moving files to Trash usually keeps their disk blocks allocated.
Do not claim the size is guaranteed free space,
and do not collapse different risks into one generic "cache" group. Keep raw
paths within the report-processing scope agreed before analysis. A conversation
with a hosted model is not local storage; do not expose new paths merely to
complete this summary without permission for that scope.

After selected groups, add a short "Not included" section with counts and sizes
for `available`, `review`, `suppressed`, and `excluded` candidates that are not
selected, plus global `partial`, `no-known-location`, and `os-managed` coverage.
Explain the main decision codes without dumping every item.

Finish with the exact scan roots, plan path, SHA-256, selected total, application
quit requirements, and one question in this form. When applications must be
closed, make confirming that fact part of the same question:

> After closing APPLICATIONS, confirm moving the N items above (SIZE) to the
> system trash using plan PATH, SHA-256 DIGEST?

## Accept exact authorization

Accept authorization only when it is an unambiguous answer to the single
confirmation question after the plan summary. Because that question embeds the
plan path and SHA-256, a direct affirmative answer clearly refers to them; do
not require the user to repeat or copy the digest. Never infer authorization
from the initial request, "clean whatever you think is safe", a broad
permission, an automation default, a recommendation, or a third party.

Authorization expires if the scope, config, plan, or digest changes.

## Execute

Run exactly:

```bash
cleanr clean \
  --plan /local/path/cleanr-plan.json \
  --plan-sha256 <authorized-sha256> \
  --authorized-by-user
```

Cleanr verifies the digest, re-scans the roots, rebuilds the deterministic
plan, compares the selected execution projection and safety provenance,
validates every target, moves successful items to system trash, and writes an
execution manifest. Drift limited to unselected candidates does not invalidate
the reviewed actions. A refusal or selected-target/safety change requires a new
analysis, plan, summary, and authorization. Never fall back to another cleanup
tool.

Report the run ID and every success or failure. Do not empty the trash.

## Restore

Use `cleanr restore list` for read-only review. Only when the current user
explicitly names a run to restore, run:

```bash
cleanr restore run <run-id> --confirm
```

Report restored, skipped, and failed items. Never overwrite an existing restore
target or bypass Cleanr's checks.
