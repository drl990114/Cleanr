---
description: A contributor-oriented map of Cleanr's crates, data flow, and safety boundaries.
---

# Architecture

This page is for contributors and plugin authors who need to understand where
Cleanr's behavior lives. If you only want to use the application, start with
[Using Cleanr](./using-cleanr.md).

## Workspace crates

| Crate | Path | Responsibility |
| --- | --- | --- |
| `cleanr-core` | `crates/core` | Scan entries, rule hits, evidence reports, cleanup plans, safety policy, and manifest models |
| `cleanr-cli` | `crates/cli` | Command-line adapters, argument parsing, read-only output, distribution commands, and plugin management |
| `cleanr-tui` | `crates/tui` | Interactive terminal application, state machine, views, and background workflow adapters |
| `cleanr-fs` | `crates/fs` | Filesystem scanning, metadata collection, cancellation, and `ScanReport` generation |
| `cleanr-rules` | `crates/rules` | Built-in and plugin rule loading, validation, matching, and the `RuleRegistry` |
| `cleanr-plugin-api` | `crates/plugin-api` | Versioned manifests, discovery, compatibility, trust, schemas, and diagnostics |
| `cleanr-config` | `crates/config` | Configuration schema, defaults, validation, and atomic persistence |
| `cleanr-i18n` | `crates/i18n` | Pure language-pack parsing, validation, fallback, and runtime locale switching |
| `cleanr-tasks` | `crates/tasks` | Shared scan/evidence/plan workflow, guarded cleanup entry points, restore, platform trash integration, and manifest persistence |

## Runtime data flow

```text
CLI or TUI adapter + config
             │
             ▼
      cleanr-tasks workflow
 resolve scope → scan → rules → evidence → plan
       │          │       │         │
       │          │       │         └── cleanr-core
       │          │       └──────────── cleanr-rules
       │          └──────────────────── cleanr-fs
       ▼
       user review
       │          │
       │          └── delegated: exact digest + re-scan + provenance check
       └───────────── local TUI: explicit confirmation
                         │
                         ▼
pending manifest → target revalidation → system trash → manifest update
                         │
                         └──────────────→ restore → restore manifest
```

The plan builder removes overlapping candidates before it computes selected and
total selected file bytes, not an observed free-space increase.

The entry-only `build_cleanup_plan*` functions remain deprecated compatibility
APIs. Product code must retain scan integrity and provenance through the
analysis-backed builder used by the shared workflow.

The shared workflow exported by `cleanr-tasks` is the only product-facing orchestrator for scope
resolution, scanning, rule annotation, evidence generation, and plan creation.
The CLI and TUI adapt arguments, progress, and presentation; they do not compose
those lower-level crates independently.

## Internal module boundaries

- `cleanr-core` separates serialized models, evidence, planning, safety policy,
  and execution/restore manifests;
- `cleanr-fs` separates scope discovery, scan traversal, budget accounting, and
  platform file identity;
- `cleanr-rules` separates schemas, plugin loading, registry/index ownership,
  and matching;
- `cleanr-tasks` separates workflow orchestration, cleanup, restore, manifest
  storage, and operating-system adapters.

`node scripts/check-architecture.mjs` guards these boundaries in CI. It rejects
CLI or TUI calls that bypass the shared workflow, a public raw cleanup executor,
and distribution-only network dependencies in `cleanr-i18n`.

## TUI boundaries

`cleanr-tui` keeps rendering separate from I/O:

- `app/` owns state transitions and user actions;
- `effects/` owns background scanning, persistence, cleanup, and restore work;
- `views/` renders immutable application state;
- `commands/` maps action requests to palette entries;
- `terminal.rs` owns raw mode, input polling, drawing, and terminal cleanup.

Views do not walk the filesystem. Background workers report results back to the
state machine, which keeps cancellation and partial failure visible to the UI.

## External local AI boundary

`cleanr analyze` is a CLI-only, read-only boundary for an external agent on the
same machine. It scans, applies the deterministic rule and recommendation
policy, and prints a versioned `AnalysisReport` JSON document. It does not
create a cleanup plan, grant authorization, or move files. An agent may use
that evidence to explain or propose a review, while the user still selects and
confirms cleanup in Cleanr.

The report includes raw local paths, scan roots, rule metadata and explanatory
text, and diagnostics. It is deliberately a local contract rather than a
remote transport object; a future remote-sharing feature would require a
separate redacted DTO and threat model.

## Safety boundaries

Safety is enforced in more than one layer:

- `cleanr-rules` limits automatic selection to high-confidence trusted rules;
- `cleanr-core` excludes protected and overlapping candidates while building
  the plan and records directory fingerprints for selected trees;
- `cleanr-tasks` exposes separate local-confirmation and delegated-cleanup
  entry points, keeps the raw executor crate-private, journals cleanup before
  moving files, and revalidates each target at execution time;
- delegated cleanup binds authorization to the exact reviewed SHA-256 digest,
  reconstructs the saved scan scope and recommendation policy, re-scans, and
  rejects plan or provenance drift before execution;
- the trash backend records rollback information where the platform supports
  it;
- `cleanr analyze` is read-only and cannot mint cleanup authorization or invoke
  cleanup;
- no embedded model or provider receives scan evidence through this interface.

Plugins remain declarative by default. Their manifests, rules, and translations
are parsed as data; dynamic hooks are a separately trusted external-command
capability.

## Persistent data

Configuration uses the platform config directory. Cleanup and restore
manifests use the platform state directory under `cleanr/`, with separate
`runs/` and `restores/` folders.

`cleanr-tasks` owns manifest persistence through `ManifestRepository`, which
keeps listing, lookup, and atomic writes behind one API for the TUI and CLI.

Writes use temporary files and atomic replacement so a partial write does not
silently replace a valid config or manifest.

An agent can run tools locally while sending their output to a cloud model.
The approval flag is a caller assertion, not independent human authentication
or an OS sandbox. See [Evidence and privacy](./evidence-and-privacy.md).
