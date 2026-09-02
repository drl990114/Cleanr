---
sidebar_position: 7
description: Install, publish, or author Cleanr plugins for cleanup rules, translations, and trusted dynamic candidate hooks.
---

# Plugins

Cleanr plugins are versioned bundles. The default model is declarative: plugins
add cleanup rules and translations through data files, so Cleanr can validate
them before loading. A plugin may also declare `dynamic-candidates` hooks, but
hook execution is a separate trusted capability and is not enabled by ordinary
installation.

Plugins may also declare `scan-locations`. These are constrained, relative
locations anchored to `home`, `cache`, `data-local`, `data`, `temp`, or
`downloads`. They cannot contain absolute paths, parent traversal, or globs.
Only built-in or explicitly trusted plugins may activate them; untrusted
locations are diagnosed and ignored.

## Package Manager

Install from the official static index:

```bash
cleanr plugin search cache
cleanr plugin install example.caches
cleanr plugin list
cleanr plugin update
```

Install from another GitHub repository or static index:

```bash
cleanr plugin install example.caches \
  --github-repo owner/repo \
  --github-ref main

cleanr plugin install example.caches \
  --index-url https://example.com/plugins/index.json
```

Useful management commands:

```bash
cleanr plugin info example.caches
cleanr plugin remove example.caches
cleanr plugin trust example.caches
cleanr plugin untrust example.caches
cleanr plugin doctor
```

By default, Cleanr installs into the platform config directory under
`cleanr/plugins`, records the plugin's source index for future updates, adds the
plugin directory to `[plugins].dirs`, and enables rule packs declared by the
plugin. Use `--trust` only after reviewing the bundle; trusted high-confidence
rules may preselect cleanup items.

## Local Development

Scaffold a plugin:

```bash
cleanr plugin init ./plugins/example-caches \
  --id example.caches \
  --name "Example cache rules"
```

Validate and link it into your local Cleanr config:

```bash
cleanr plugin validate ./plugins/example-caches
cleanr plugin link ./plugins/example-caches
cleanr plugin unlink example.caches
```

Generate editor schemas:

```bash
cleanr plugin schema manifest > plugin.schema.json
cleanr plugin schema index > plugin-index.schema.json
cleanr plugin schema rules > rules.schema.json
cleanr plugin schema locations > locations.schema.json
cleanr plugin schema language > language.schema.json
cleanr plugin schema config > config.schema.json
```

## Official Index

The official index is a static JSON file at `plugins/index.json`. Each entry
contains plugin metadata plus every downloadable file's URL, byte size, and
SHA-256 hash. Cleanr downloads into a staging directory, verifies all hashes,
validates the bundle, then atomically swaps it into place.

Built-in rule packs are stored separately under `crates/rules/builtin-plugins/`
and compiled into Cleanr. They are not listed in `plugins/index.json` unless a
future release intentionally offers them as downloadable plugins too.

Generate or check an index:

```bash
cleanr plugin index \
  --plugin-dir plugins \
  --base-url https://raw.githubusercontent.com/owner/repo/main/plugins

cleanr plugin index --check
```

GitHub PRs are the recommended publishing path:

1. Add a bundle under `plugins/<bundle-name>/`.
2. Run `cleanr plugin validate plugins/<bundle-name>`.
3. Run `cleanr plugin index --check` or regenerate `plugins/index.json`.
4. Open a PR with the plugin files and generated index.

npm packages or crates can still host the same `plugins/` directory through a
static HTTP URL, but Cleanr's installer intentionally consumes the stable JSON
index format instead of registry-specific archives.

## Minimal Bundle

```text
example-caches/
├── plugin.toml
├── locations/
│   └── global.toml
└── rules/
    └── caches.toml
```

```toml title="plugin.toml"
api_version = "1"
id = "example.caches"
name = "Example cache rules"
version = "1.0.0"
description = "Cleanup rules for Example Tool caches."
cleanr_version = ">=0.1.0"
capabilities = ["rules", "scan-locations"]
categories = ["developer"]
keywords = ["cache"]
```

```toml title="rules/caches.toml"
id = "example-caches"
name = "Example caches"
version = "1.0.0"
description = "Generated caches for Example Tool."
categories = ["developer-cache"]

[[rules]]
id = "example-cache"
label = "Example Tool cache"
category = "developer-cache"
match = { dir_name = ".example-cache", min_size = 1048576 }
confidence = "high"
default_selected = true
action = "trash"
reason = "Example Tool recreates this cache automatically."
risk_note = "The next Example Tool run may be slower."
```

```toml title="locations/global.toml"
id = "example-global-locations"
version = "1.0.0"

[[locations]]
id = "example-linux-cache"
label = "Example Tool cache"
kind = "app-caches"
platforms = ["linux"]
base = "cache"
relative_path = "example-tool"
```

Use `mode = "os-managed"` for path-free coverage that Cleanr should explain
but never traverse or place in a cleanup plan.

## Trust and Hooks

New plugins are untrusted by default. Their candidates are visible, but they
cannot preselect items even when a rule declares `default_selected = true`.

```toml
[plugins]
trusted = ["example.caches"]
```

The trusted ID is the plugin manifest ID, not the rule-pack ID. Trust does not
bypass path validation, protected paths, trash behavior, or local user
authorization.

Dynamic hooks are declared in `plugin.toml` with the `dynamic-candidates`
capability. Current releases validate those declarations but do not execute hook
commands while loading rules. The future runtime will treat hooks as explicit
external commands with JSON stdin/stdout, timeouts, and host-side validation.
Install, pre-cleanup, and post-cleanup hooks remain out of scope for the first
hook runtime milestone.
