# Global coverage review

Read this file for every global or multi-category cleanup request. A broad user
goal is not permission to scan every known category. Keep the approved scope
explicit and local.

## Map the request to Cleanr categories

Use only these user-level categories:

- `developer-caches`: rebuildable package-manager and tool caches such as npm,
  pnpm, Yarn, Cargo, pip, Gradle, Go, and supported Apple developer locations.
  A known location is evidence of coverage, not evidence that its contents are
  safe to select. Explain re-download, rebuild, and offline-use risks. Never run
  a package manager's native prune, verify, or clean command.
- `browser-caches`: supported browser cache locations. Review only candidates
  for rebuildable cache, code cache, or GPU cache data. Never select a profile,
  cookie, history, password, service worker, login token, or saved session.
- `app-caches`: supported user-level application caches. Record the affected
  applications so the final confirmation can require that they are closed.
- `logs`: supported user-level logs. System journals and system-owned logs stay
  outside Cleanr's delegated workflow.
- `temp-files`: supported user temporary locations. A scan root is never itself
  a cleanup candidate.
- `downloads`: personal files, not routine junk. Include only after the user
  explicitly asks for Downloads review.

Windows Update, Delivery Optimization, staged macOS updates, Time Machine or
other snapshots, system package-manager state, the Recycle Bin or Trash, and
system-owned roots are `os-managed`. Explain that Cleanr will not act on them.
Do not invoke an OS or package-manager cleanup command as a fallback.

Use the same explicit categories on macOS and Linux. On Windows, keep a routine
review to `app-caches` and `temp-files`; add `developer-caches` or
`browser-caches` only when the user separately includes them. Example for an
approved cross-platform cache review:

```bash
cleanr analyze --global \
  --global-kind developer-caches \
  --global-kind browser-caches \
  --global-kind app-caches \
  --global-kind logs \
  --global-kind temp-files
```

Remove every category the user did not approve. Do not add `downloads` by
default.

## Apply platform defaults

- On macOS, a routine review may include `browser-caches`, `app-caches`,
  `logs`, and `temp-files`. Add `developer-caches` only when the user includes
  Homebrew, package-manager, or Xcode caches. Exclude Trash, Mail, iOS backups,
  Time Machine snapshots, browser service workers, and system-owned roots.
- On Linux, add only the categories named by the user. Keep system journals,
  system package-manager state, snapshots, Trash, and system-owned roots
  outside delegated cleanup.
- On Windows, keep a conservative routine to `app-caches` and `temp-files`.
  Its automatically selectable system rules are limited to
  `windows-stale-directx-shader-cache-file` and
  `windows-stale-user-temporary-file`, which match stale regular files rather
  than the containing directories. Ask separately before browser or developer
  caches. Explorer thumbnail databases, system-owned crash dumps, Windows
  Update, Delivery Optimization, Prefetch, Downloads, registry data, the
  Recycle Bin, and system-owned roots stay outside the routine. Locked files
  remain in place as recorded failures.

Do not interrupt early to ask the user to quit an application. Add the affected
applications to the final human-readable plan summary and make quitting them a
condition of the single execution confirmation. Cleanr does not stop
applications.

## Build the coverage ledger

For global analysis, read `scan.global.requested_kinds`,
`scan.global.locations`, and `scan.global.os_managed`. Each location has a
`kind`, human-readable `label`, `local_path`, and the parent-deduplicated
`scan_root` that covered it. Each OS-managed entry has only an `id`, `kind`,
and human-readable `label`: it deliberately contains no path and can never be
a scan root or candidate. Keep raw paths local.

Report one row for every approved Cleanr category and every requested
OS-managed item. Use exactly one of these states:

- `found-candidates`: at least one report candidate belongs to the category.
- `checked-empty`: the complete scan covered at least one named location for
  the category and found no category candidate.
- `no-known-location`: the category was requested but `locations` contains no
  existing named location for it.
- `partial`: report integrity is partial, or an issue leaves the category's
  covering root incomplete. Never call a partial category empty.
- `os-managed`: the requested item is outside Cleanr's recoverable execution
  boundary.

Apply the states conservatively:

1. If `scan.global` is absent for a claimed global analysis, coverage is
   unavailable. Remain read-only and rerun with a current Cleanr CLI.
2. Verify that `requested_kinds` equals the approved Cleanr categories. Treat a
   missing or extra category as a scope mismatch.
3. Give `partial` priority over candidate or empty states.
4. For each candidate, find all containing named locations and assign it to the
   location or locations with the longest `local_path`. Use their `kind`; do
   not guess a category from a label or broad parent root.
5. Use `found-candidates` when the category has assigned candidates. Otherwise
   use `checked-empty` only when at least one named location exists and its
   coverage is complete; use `no-known-location` when none exists.
6. Use `os-managed` only for entries explicitly present in
   `scan.global.os_managed`; do not infer this state from a broad parent root or
   invoke a native cleanup command to fill a coverage gap.
7. List excluded, unrequested, and OS-managed items separately. Never describe
   them as cleaned or checked-empty.

For each `found-candidates` row, summarize count, size, recommendation states,
decision codes, and the main rebuild or application risk. A coverage state is
not cleanup authorization. `checked-empty` means no matching candidate in the
known covered locations under current rules and policy; it does not mean the
computer has no junk.

## Preserve the execution boundary

Only Cleanr candidates can enter a plan. Never select a named location or scan
root merely because it appears in the coverage ledger. Keep `partial`,
`no-known-location`, and `os-managed` rows out of plans. Preserve the main
skill's exact plan path, SHA-256, fresh re-scan, system-trash, manifest, and
current-user authorization requirements.
