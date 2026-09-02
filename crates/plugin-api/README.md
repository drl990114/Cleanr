# cleanr-plugin-api

Shared declarations for cleanr's data-only plugin system: manifests,
capabilities, deterministic discovery, trust levels, and load diagnostics.

Plugins are declarative bundles and never load native or WebAssembly code. A
bundle contains a `plugin.toml` manifest plus `rules/*.toml`,
`locations/*.toml`, and/or `locales/*.yml`. Scan locations are constrained to
known platform directory anchors and activate only for built-in or trusted
plugins. Discovery order is deterministic, duplicate IDs are rejected, and
invalid bundles are reported without aborting application startup.
