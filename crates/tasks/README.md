# cleanr-tasks

Cleanup execution and restore manifest management for cleanr.

This crate executes a `CleanupPlan`, moves items to the system trash, writes
execution manifests, restores cleanup runs on supported desktop platforms, and
writes separate restore manifests for audit and retry. Cleanup requires an
explicit authorization value, validates that the plan is recoverable, and
records the authorization source in the execution manifest.

`ManifestRepository` is the central entry point for state-directory manifest
I/O. The free functions remain as compatibility wrappers.
