# Cleanr Plugins

This directory is the default source for Cleanr's public plugin index. Plugins
here are declarative bundles that Cleanr can validate, install, update, and load
without executing native or WebAssembly code by default. A bundle contains a
`plugin.toml` manifest plus optional `rules/*.toml`, `locations/*.toml`, and
`locales/*.yml` files. Location packs activate only after the bundle is trusted
and use constrained platform directory anchors rather than arbitrary paths.

Built-in rule packs intentionally live under `crates/rules/builtin-plugins/`
instead. They are compiled into release binaries and are not listed in the
downloadable plugin index unless explicitly copied here later.
