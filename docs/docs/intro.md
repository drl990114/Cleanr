---
sidebar_position: 1
description: Understand what Cleanr cleans, how it keeps cleanup reviewable, and where to begin.
---

# Cleanr overview

Cleanr is an AI-friendly, cross-platform disk cleanup tool for **application and browser caches,
logs, temporary files, downloads, and development artifacts**. It runs in your
terminal, explains why each item matched, and lets you decide what moves to
system Trash. Its structured, read-only analysis gives AI agents candidate sizes,
matching reasons, confidence, and risk notes, so they can help you review and
prepare a plan. The final cleanup decision stays with you.

Safety checks are part of that workflow: analysis is read-only, confirmation is
on by default, and selected targets are revalidated before each move. Agent
execution verifies the reviewed plan against a fresh scan. Cleanup uses system
Trash and local records for best-effort recovery.

Typical candidates include:

- browser and application caches from known user-level cleanup locations;
- logs, diagnostic reports, and temporary files;
- downloads and installers that need your judgment before removal;
- project dependencies such as `node_modules`;
- build output such as Rust `target` directories and Xcode `DerivedData`;
- package-manager caches for Cargo, npm, pnpm, pip, Gradle, and other tools.

Choose directories or known cleanup locations by category. Coverage depends on
your platform and selected scope; see [scanning options](./using-cleanr.md).

## The basic workflow

Cleanr separates discovery from deletion:

1. **Scan** one or more directories.
2. **Review** the matched candidates and their risk notes.
3. **Select** only the items you want to remove.
4. **Confirm** the cleanup.
5. **Restore** a previous cleanup run if needed.

Nothing is cleaned just because it was found. High-confidence, rebuildable
items may be preselected, but the plan remains editable before execution.

## Safety at a glance

- Cleanup moves items to the operating system trash; it does not permanently
  delete them.
- Cleanr removes overlapping parent and child candidates before calculating
  candidate and selected-byte totals. Moving items to Trash usually does not
  immediately increase free disk space.
- Targets are checked again immediately before cleanup. Changed files,
  symbolic links, paths outside the scan roots, and protected Cleanr data are
  rejected.
- Each cleanup and restore writes a local manifest so the result is auditable.
- Plugins are declarative data files by default; dynamic hooks require explicit trust.

See [Safety and recovery](./safety-and-recovery.md) for the exact guarantees and
restore limitations.

## Is Cleanr a good fit?

Cleanr suits people who want to review disk cleanup from a keyboard-driven
interface, whether they are inspecting application caches, old downloads, or
project build output. It is not a general-purpose system optimizer, registry
cleaner, or unattended deletion service.

## Start here

- [Quick start](./quick-start.md): install Cleanr and complete a read-only first review.
- [Using Cleanr](./using-cleanr.md): learn the workflow, shortcuts, and commands.
- [Evidence and privacy](./evidence-and-privacy.md): use the local, read-only
  analysis contract with an agent, including the cloud-provider data boundary.
- [Configuration](./configuration.md): change scan, cleanup, language, and theme.
- [Troubleshooting](./troubleshooting.md): resolve common startup, scan, and
  restore problems.

See [Release readiness](./support-matrix.md) for version and platform verification limits.
