---
sidebar_position: 5
description: Configure Cleanr scan roots, ignores, cleanup confirmation, plugins, language, and theme.
---

# Configuration

Cleanr uses a strictly validated TOML file. You usually do not need one for a
first run because sensible defaults are built in.

## Find or create the config

Print the default platform-specific path:

```bash
cleanr config path
```

Create the default file without overwriting an existing one:

```bash
cleanr config init
```

Use `--force` only when you intentionally want to replace the existing file:

```bash
cleanr config init --force
```

To use a different file for one invocation, pass the global option:

```bash
cleanr --config ./cleanr.toml ~/projects
```

## Default configuration

```toml
[scan]
stay_on_filesystem = false
ignore_dirs = []
ignore_patterns = ["**/.git", "**/.git/**"]
global_kinds = ["developer-caches", "browser-caches", "app-caches", "temp-files", "logs", "downloads"]

[cleanup]
default_action = "trash"
require_confirm = true
enabled_rule_packs = ["builtin-dev", "builtin-general", "builtin-system"]

[recommendations]
preselect_after_days = 90

[plugins]
# dirs defaults to the platform config directory under cleanr/plugins
trusted = []

[i18n]
# locale defaults to LC_ALL, LC_MESSAGES, LANG, then en-US
# locale = "zh-CN"
# dirs defaults to the platform config directory under cleanr/languages

[ui]
# "auto" detects the terminal background; "dark" and "light" are explicit
theme = "auto"
```

## Change common values from the CLI

You can edit the TOML file directly or use dotted keys:

```bash
cleanr config get ui.theme
cleanr config set ui.theme dark
cleanr config set scan.stay_on_filesystem true
cleanr config set cleanup.require_confirm false
cleanr config set recommendations.preselect_after_days 180
cleanr config set i18n.locale zh-CN
```

Supported values include `true`/`false`, `yes`/`no`, `on`/`off`, and `1`/`0`
for booleans. An unknown key or invalid value is rejected without replacing a
valid configuration.

## Configuration reference

### `[scan]`

| Option | Default | Description |
| --- | --- | --- |
| `stay_on_filesystem` | `false` | When `true`, do not cross filesystem boundaries during a scan |
| `ignore_dirs` | `[]` | Exact directory paths to skip |
| `ignore_patterns` | Git metadata globs | Glob patterns matched against absolute and root-relative paths |
| `global_kinds` | all built-in kinds | System cleanup categories used by `/scan --global` |

Use `ignore_dirs` for known absolute directories and `ignore_patterns` for
repeatable names or layouts:

```toml
[scan]
ignore_dirs = ["/home/me/projects/large-fixture"]
ignore_patterns = ["**/.git/**", "**/vendor/**", "**/.venv/**"]
```

### `[cleanup]`

| Option | Default | Description |
| --- | --- | --- |
| `default_action` | `"trash"` | Cleanup action; currently only `"trash"` is supported |
| `require_confirm` | `true` | Ask for confirmation before a direct local cleanup |
| `enabled_rule_packs` | built-in packs | Rule pack IDs to load |

Disabling confirmation changes the dialog only. The execution layer still
requires a local user action; see [Safety and recovery](./safety-and-recovery).

### `[recommendations]`

| Option | Default | Description |
| --- | --- | --- |
| `preselect_after_days` | `90` | Observed modification-age threshold for deterministic preselection; `0` disables the age gate and values from `1` through `3650` are accepted |

This is one shared policy: the TUI, `cleanr analyze`, `cleanr plan`, and
`cleanr dry-run` all use it. The age is based on observed modification metadata,
not proven last access. Missing, future, partial, or incomplete evidence still
blocks automatic preselection.

## External local AI tools

Cleanr has no embedded model, provider, endpoint, or API-key configuration.
An external agent running on the same machine can consume the read-only
`cleanr analyze` JSON contract, but analysis grants no cleanup capability by
itself. Delegated cleanup requires a separately reviewed plan, its SHA-256, and
explicit current-user authorization. The report includes the configured
recommendation-policy snapshot and real local paths, so it is not a safe
remote-sharing format.
See [Evidence and privacy](./evidence-and-privacy) before giving it to another
tool.

### `[plugins]`

| Option | Default | Description |
| --- | --- | --- |
| `dirs` | platform Cleanr plugin directory | Directories containing plugin bundles or legacy rule files |
| `trusted` | `[]` | Plugin IDs allowed to preselect high-confidence candidates |

See [Plugins](./plugins) before trusting a third-party bundle.

### `[i18n]`

| Option | Default | Description |
| --- | --- | --- |
| `locale` | environment, then `en-US` | Active locale such as `en-US` or `zh-CN` |
| `dirs` | platform Cleanr language directory | Directories containing language YAML files |

`cleanr init --locale zh-CN` installs a built-in language file and updates
these settings.

### `[ui]`

| Option | Default | Description |
| --- | --- | --- |
| `theme` | `"auto"` | `"auto"`, `"dark"`, or `"light"` |

## Validation errors

Cleanr rejects unknown fields, unsupported enum values, empty IDs, and
duplicate trusted plugin or enabled rule-pack IDs. If Cleanr will not start
after an edit, run it with the same `--config` path and read the reported field
or value; the existing file is not silently repaired.
