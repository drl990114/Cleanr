# cleanr-tasks

Shared product workflow, cleanup execution, and restore management for Cleanr.

`workflow` is the shared CLI/TUI orchestration layer for configuration, scope
resolution, filesystem scanning, rule annotation, evidence generation, and
cleanup-plan creation. Callers adapt arguments, progress, and presentation
instead of rebuilding that sequence themselves.

Cleanup has two product-facing entry points:

- `execute_locally_confirmed_plan` is for the TUI immediately after its local
  confirmation dialog;
- `execute_delegated_cleanup` requires explicit delegation for an exact plan
  digest, re-scans the saved scope, rebuilds the plan, and rejects drift before
  execution.

The raw executor is crate-private. Both paths validate recoverability, journal
before mutation, revalidate targets, use system trash, and record authorization
in the execution manifest. Restore remains a separate audited operation.

`ManifestRepository` is the central entry point for state-directory manifest
I/O. The free functions remain as compatibility wrappers.
