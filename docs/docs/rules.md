---
sidebar_position: 6
description: Understand why Cleanr marks a path, how confidence affects selection, and what built-in rules cover.
---

# Rules and confidence

Cleanr does not decide that a path is removable from its name alone. It matches
scanned entries against versioned **rule packs** that explain what the item is,
why it can be removed, and what rebuilding it may cost.

## What you see for each candidate

Each rule match includes:

| Field | Meaning |
| --- | --- |
| Label | Human-readable name, such as “Rust target directory” |
| Category | Group such as `build-cache`, `package-cache`, or `downloads` |
| Confidence | `High`, `Medium`, or `Low` |
| Reason | Why the path is considered a cleanup candidate |
| Risk note | What may break, slow down, or require a download afterward |
| Default selection | Whether the rule asks to preselect the item |
| Match role | `primary` for a specific rule, or `fallback` for broad evidence |

When multiple rules match one entry, Cleanr retains every match as evidence.
Rules with equivalent safety semantics resolve deterministically. A trusted
specific `primary` rule may shadow a broad `fallback` rule for the selection
decision, while the fallback remains visible in the report and plan. An
untrusted rule cannot shadow a built-in fallback. Other semantic disagreements
remain an unresolved conflict and require review instead of being ranked away.
The final plan removes overlapping parent and child candidates so space is not
counted twice.

## Confidence is not a guarantee

| Level | How to treat it |
| --- | --- |
| `High` | Usually generated or downloadable data; still review unfamiliar paths |
| `Medium` | Often rebuildable, but may be expensive or contain local-only state |
| `Low` | May be user data and always needs careful manual review |

Only a `High` confidence rule with `default_selected = true` from a built-in or
trusted source can preselect an item.

Bulk selection changes only `Preselected` and `Available` items. `Review`
items, including unresolved rule conflicts, must be selected individually.

## Built-in rule packs

### `builtin-dev`

The built-in plugin manifest `cleanr.builtin.dev` provides the `builtin-dev`
rule pack. In addition to known package-manager and tool caches, the pack uses
project-aware rules for generated project artifacts. These rules first identify
a project root from marker files, optionally constrained by its direct child
directories, then match only declared, exact paths relative to that root. A
directory name alone is not enough to identify one of these project artifacts.

Project-aware coverage includes:

- Cargo, Node.js and React Native, Unity, Haskell, SBT, Maven, Gradle, CMake,
  and Unreal Engine;
- Jupyter, Python, Pixi, Composer, Pub, Flutter, Elixir, Swift, Zig, Godot,
  and .NET;
- Turborepo, Terraform, and CocoaPods.

The pack also retains rules for caches such as Cargo registries and Git
dependencies, npm, pnpm, Yarn, pip, uv, Go modules, Xcode `DerivedData`, and
Next.js and Python tool caches. On macOS it also discovers Homebrew, CocoaPods,
SwiftPM, Go build, Deno, Cypress, Composer, Bun, Pub, CoreSimulator, and other
named Xcode caches. DeviceSupport and XCTest devices require review; Xcode
archives are low-confidence because retained builds and dSYMs may be
irreplaceable.

Python `.venv` directories are intentionally not covered: they may contain
local environments that are costly or impossible to reproduce exactly. Other
higher-risk or potentially locally stateful directories are review-only and
are never preselected; read their reason and risk note before including them
in a cleanup plan.

### `builtin-general`

Finds broader candidates that should be reviewed manually:

- files of at least 100 MiB under a Downloads directory;
- `.log` files of at least 50 MiB;
- `.tmp` files of at least 1 MiB.

These rules are intentionally medium or low confidence and start unselected.

### `builtin-system`

Finds known user-level system cleanup candidates:

- browser cache directories for Chrome, Chromium, Edge, Firefox, Safari,
  Brave, and Arc;
- the standard macOS application-cache root plus narrowly named cache
  directories for popular desktop apps when they live under Application
  Support or an app container;
- Quick Look thumbnails, Zoom update installers, user logs, and diagnostic
  reports;
- stale regular files in the current Windows user's Temp and DirectX
  `D3DSCache` directories;
- large temporary files and Downloads files, including `.dmg`, `.pkg`,
  `.mpkg`, and `.iso` installers.

Only known rebuildable cache targets may be preselected, and they still pass
the shared age and evidence gates. Broad application caches, Spotify's
persistent cache, logs, diagnostics, generic temporary-file matches, and
Downloads remain review-only. Quit an application before selecting its cache.

The macOS allowlist was audited against
[Dusty](https://github.com/yagcioglutoprak/dusty) and
[PureMac](https://github.com/momenbasel/PureMac), then narrowed to preserve
Cleanr's trash-and-restore model. Cleanr deliberately excludes Trash contents,
Mail data, iOS backups, Time Machine snapshots, browser service workers, Docker
prune actions, and system-owned roots.

The Windows allowlist is intentionally file-only. A Windows-specific rule
requires at least 30 days without modification before matching:

- **user temporary file** means a regular file below the current user's
  `AppData\Local\Temp`; the Temp directory and child directories are not
  candidates;
- **DirectX shader cache file** means a regular generated graphics-cache file
  below `AppData\Local\D3DSCache`; Windows recreates it as needed, although the
  next graphics launch may spend time recompiling shaders.

Cleanr does not stop applications. If Windows keeps a candidate locked, moving
that item to the Recycle Bin fails and the original stays in place. Explorer
thumbnail databases are excluded because established cleaners restart
Explorer to release them. Crash dumps, Windows Update and Delivery Optimization
data, Prefetch, the Recycle Bin, registry data, Downloads, and system-owned
roots are also excluded from this conservative Windows routine.

The Windows paths were audited against
[BleachBit](https://github.com/bleachbit/bleachbit/tree/ab0e4b94e29b8233adbe7ab010656e61b162c63d)
and
[Winapp2](https://github.com/MoscaDotTo/Winapp2/tree/3c0156de665cc180edc76745e425412ccc4356ca),
then independently narrowed using Microsoft's descriptions of
[Storage Sense temporary-file cleanup](https://learn.microsoft.com/windows/client-management/mdm/policy-csp-storage#allowstoragesensetemporaryfilescleanup)
and
[generated DirectX and thumbnail caches](https://techcommunity.microsoft.com/blog/filecab/creating-remediation-actions-for-system-insights/428234).
No external cleaner database or executable is bundled. Platform-specific scan
roots are registered only by the corresponding operating-system build; the
shared `builtin-system` plugin supplies their declarative explanations.

## Enable or disable packs

Only IDs in `cleanup.enabled_rule_packs` are loaded:

```toml
[cleanup]
enabled_rule_packs = ["builtin-dev", "builtin-general", "builtin-system"]
```

Removing `builtin-general` and `builtin-system` is useful when you want Cleanr
to focus only on developer caches.

Run `/rules` inside the TUI to inspect the active packs and rules.

## Add custom rules

The recommended format is a declarative plugin bundle. See
[Plugins](./plugins) for a complete minimal example, validation commands, and
the trust model.

For generated paths that are meaningful only inside a particular project, use
a project matcher instead of a broad directory-name or path glob. Positive
marker and root-directory globs identify the project root, excluded globs veto
ambiguous roots, and `artifact_paths` lists the exact relative directories that
may match:

```toml
[rules.match]
kind = "directory"

[rules.match.project]
marker_globs = ["acme-project.toml"]
root_dir_globs = ["src"]
excluded_marker_globs = ["acme-keep-build"]
excluded_root_dir_globs = ["keep-output"]
artifact_paths = ["build/cache", "build/generated"]
```

This fragment belongs to a `[[rules]]` entry. Keep the usual confidence,
default-selection, reason, and risk fields conservative, especially when an
artifact may require network access or contain local-only state. Excluded globs
only veto children observed in the same scan snapshot; an ignored path is not
proof that a child does not exist, so never use an exclusion as the rule's only
safety boundary. When publishing a bundle that uses this matcher, set its
`cleanr_version` to the first Cleanr release whose rule schema supports
`project`; do not reuse the generic `>=0.1.0` minimum from the minimal example.

Legacy loose TOML rule-pack files are still discovered in plugin directories,
but bundles provide version and compatibility metadata and are preferred.

### Path glob and fallback semantics

Path globs are segment-aware on every platform: `*` matches within one path
segment and never crosses `/`, while `**` may match recursively across
segments. For example, `**/Library/Caches/*` matches a direct child of
`Caches`, but not its nested descendants. Use `**/Library/Caches/**` only when
recursive matching is intentional.

Set `match_role = "fallback"` only on a deliberately broad rule that should
apply when no trusted primary rule matches the same candidate. Fallback rules
cannot use `default_selected = true`. Prefer a specific matcher or a project
matcher whenever one can express the ownership boundary.
