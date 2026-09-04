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

Cleanup and restore hold the same OS file lock in the state directory across
history lookup, filesystem operations, and journal writes. The lock file may
remain after a run; its existence does not mean an operation is active. Do not
delete it to bypass an active operation.

Restore writes `cleanr.restore.v2` and continues reading v1 history. Each item
starts as `not-attempted`, becomes `pending` before its executor call, and is
then recorded as `restored` or `failed`. A pending outcome from an interrupted
run blocks automatic retry of that item; unrelated unattempted items can
continue. Existing cleanup-v1 statuses distinguish `skipped` (not yet attempted)
from `pending` (operation intent recorded, outcome unknown).

System Trash and manifest writes are not one atomic transaction. A journal
write failure stops further operations and reports the affected run and path,
plus the cleanup receipt locator when available. Keep that error and inspect
the original path, system Trash, and manifests before manual recovery. macOS
restore uses an atomic no-replace rename; it does not overwrite a target that
appears after the initial existence check.
