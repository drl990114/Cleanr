---
description: See the current direction for Cleanr without confusing planned work with released behavior.
---

# Roadmap

This roadmap describes direction, not a compatibility promise. For behavior
you can rely on today, use the user guide and the release notes.

## Version scope

Documentation follows the repository. Category labels and filtering, cumulative
filtered selection, and `Shift+A` global selection apply to **0.15.0 and later**.
See the [changelog](https://github.com/drl990114/Cleanr/blob/main/CHANGELOG.md)
and [release readiness](./support-matrix.md) before relying on new behavior.

## Current foundation

The project already includes:

- non-overlapping cleanup plans with accurate selected-space totals;
- system-trash cleanup and manifest-based restore on macOS, Windows, and
  Freedesktop-compatible Linux desktops;
- per-item cleanup and restore results;
- cancellable single-pass scanning, glob ignores, and known-cache discovery;
- execution-time path, type, file size, directory fingerprint,
  modification, and protected-path checks;
- separate analysis and digest-checked cleanup entry points; the approval flag
  is a caller assertion, not an OS security boundary for external tools;
- a versioned, read-only local analysis report for external local agents,
  alongside scan JSON, exact candidate-path selection, digest-bound delegated
  cleanup, dry-run, and restore commands;
- versioned, declarative plugin bundles with compatibility and trust metadata.

## Near-term: clearer control and recovery

Planned work includes:

- clearer retries for partial cleanup and restore failures;
- large-tree performance benchmarks and broader cross-platform restore tests;
- more visible access to manifest and diagnostic details from the TUI.

## Developer-cache intelligence

The project intends to deepen developer-specific guidance:

- broader cache coverage for package managers, build tools, IDEs, mobile
  toolchains, and containers;
- scoring that considers safety, reclaimable space, observed modification
  recency, and rebuild cost; it must not present modification time as proven
  last use;
- conservative, balanced, and maximum-space presets;
- explanations of how each cache is recreated and whether network access is
  required;
- validated and signed distribution for community rule packs.

## Automation

Potential automation surfaces include scheduled diagnostics and richer
machine-readable failure reports. Any execution surface must remain tied to an
explicit, locally reviewed user action.

AI is an external consumer of local evidence and a possible rule-authoring
assistant. Cleanr will not embed a model or provider, treat a suggestion as
permission, or allow unattended destructive action. Delegated execution must
remain bound to an exact reviewed plan and explicit current-user authorization.
Remote sharing, if ever considered, requires a separately designed redacted
contract.
