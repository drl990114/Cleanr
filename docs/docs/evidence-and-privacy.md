---
description: Use Cleanr's versioned local analysis report safely with external local AI tools.
---

# Evidence and privacy

Cleanr is AI-friendly by exposing deterministic local facts without embedding
a model. `cleanr analyze` is the read-only evidence boundary. A separate,
bounded `cleanr clean` command can move an exact reviewed plan to system trash
only after explicit user authorization.

## Local analysis contract

Run analysis for one or more roots:

```bash
cleanr analyze /path/to/project
```

It also accepts `--global` and repeatable `--global-kind <kind>` options for
known user-level cleanup locations. Its recommendation policy comes from the
shared configuration:

```toml
[recommendations]
preselect_after_days = 90
```

For a routine macOS review, keep developer caches separate unless the user
includes them:

```bash
cleanr analyze --global \
  --global-kind browser-caches \
  --global-kind app-caches \
  --global-kind logs \
  --global-kind temp-files
```

Use the same global-kind arguments with `cleanr plan --output` only after the
user approves that scope. Add `developer-caches` explicitly for Homebrew,
package-manager, and Xcode targets. Add `downloads` only when the user
explicitly asks to review personal files in Downloads.

For a routine Windows review, keep the default scope to the two conservative
file-only categories:

```bash
cleanr analyze --global \
  --global-kind app-caches \
  --global-kind temp-files
```

This discovers the current user's Temp and DirectX `D3DSCache` locations. The
Windows-specific rules match only regular files that have not been modified for
at least 30 days; they do not select either directory. Ask separately before
adding browser or developer caches. Crash dumps, Explorer thumbnail databases,
Windows Update data, Prefetch, Downloads, registry data, the Recycle Bin, and
system-owned roots are outside this routine scope.

Set `preselect_after_days` to `0` to disable the age gate, or to an integer
from `1` through `3650`. The TUI, `cleanr analyze`, `cleanr plan`, and
`cleanr dry-run` use this same policy.

The command writes a versioned `AnalysisReport` JSON document to standard
output. It only scans and evaluates evidence. It does **not** create a cleanup
plan, change the current TUI selection, request cleanup authorization, or move
files.

## Install the agent skill

The repository includes the cross-agent `cleanr-review-disk-cleanup` skill for
local evidence review and explicitly authorized, recoverable cleanup. Install
that skill directly from GitHub with the open
[Skills CLI](https://github.com/vercel-labs/skills):

```bash
npx skills add drl990114/cleanr@cleanr-review-disk-cleanup -g
```

The installer detects supported local agents and lets you select the targets.
The `-g` flag makes the skill available to your user account across projects;
omit it to install only in the current project. You can also target an agent
explicitly with `-a <agent-name>`.

Start a new task or session in the selected agent afterward. Invoke
`$cleanr-review-disk-cleanup` where explicit skill invocation is supported, or
ask the agent to review Cleanr disk-cleanup evidence. The skill is not tied to
Codex: it uses the portable `SKILL.md` format and can be installed into any
agent supported by Skills CLI. The skill keeps analysis read-only by default.
It permits execution only after the current user reviews a plan summary and
explicitly authorizes that exact plan and SHA-256. Execution uses system trash
and a local manifest, never permanent deletion.

## What the report means

One report has a fixed `as_of` time so age decisions are consistent at the
threshold boundary. It includes:

- schema and analysis identifiers, the policy snapshot, and completion time;
- scan roots, integrity state, and structured scan issues;
- for global analysis, the requested global categories and every existing named
  location with its category, label, local path, and covering scan root;
- each candidate's opaque report-scoped ID, local path, size, kind, and
  rollback method;
- modification-time evidence, coverage, rule matches, and overlap resolution;
- a deterministic recommendation state and decision codes explaining both a
  recommendation and a non-selection.

Modification time is observed filesystem metadata, not proof that a user last
accessed a file. For a directory, Cleanr considers the newest observed
modification time in its scanned descendants. Missing, future, partial, or
incomplete evidence blocks automatic preselection.

The optional `scan.global` object is additive to `cleanr.analysis.v1`. It is
omitted for explicit-path analysis, and older v1 reports without it still
deserialize. Do not infer global coverage from `scan.roots`: parent roots are
deduplicated for efficient scanning, while `scan.global.locations` maps each
named location to a covering root whose traversal ended naturally. Structured
scan issues still identify ignored or filesystem-boundary subtrees, so a
location row alone does not prove every descendant was read. When scan integrity is complete, a
requested category with no location means Cleanr found no known existing location for that
category; it does not mean the computer is clean. The bundled agent skill converts this evidence into
`found-candidates`, `checked-empty`, `no-known-location`, or `partial`, and uses
`os-managed` for requested system-update or other system-owned work that Cleanr
must not execute.

## Recommended external-agent workflow

1. A local agent invokes `cleanr analyze` for a user-approved scope.
2. It reads the report and proposes questions, explanations, or a review
   order.
3. If cleanup is requested, it writes a local plan with `cleanr plan --output`.
   It may use repeatable `--select` and `--deselect` only to encode exact
   candidate-path choices the current user made after reviewing the evidence;
   the plan file itself is not edited.
4. It inspects the selected trash actions and summarizes the exact roots,
   count, size, risks, plan path, and printed SHA-256.
5. The current user explicitly authorizes that exact plan after seeing the
   summary.
6. The agent runs `cleanr clean` with the plan path, reviewed SHA-256, and
   `--authorized-by-user`.
7. Cleanr verifies the digest, re-scans and compares the plan, validates every
   target, moves successful items to system trash, and records the manifest.

The analysis command has no cleanup operation. A suggestion, recommendation,
initial cleanup request, or broad standing permission is never an execution
token. An agent must not select a review-only item on its own judgment. Unknown,
overlap-suppressed, and safety-excluded paths cannot be selected. If the plan
changes, the agent must generate, summarize, and obtain authorization for a new
plan.

## Data boundary

`AnalysisReport` and cleanup plan files are intentionally **local** contracts.
They contain raw local paths, scan roots, rule reasons and risk notes, and issue
paths. Cleanr has no embedded AI provider, API-key setting, prompt transport,
or telemetry that sends them elsewhere.

## Budget-limited evidence

When a scan budget is reached, `scan.budget_exceeded` contains path-free limit and observed
values, report integrity is `partial`, and candidate coverage is `unknown`. The already collected
local evidence remains useful for review, but it is read-only: Cleanr refuses to create a cleanup
plan from it. A later complete scan is required before planning or cleanup.

Do not forward the JSON to a remote service as-is. If you choose to share any
of it, you are responsible for minimizing the scope and removing sensitive
details. A safe remote-sharing feature would need a separate redacted DTO and
an explicit threat-model review; the local report is not that DTO.
