# cleanr-rules

Rule engine and registry for cleanr.

This crate parses rule packs, validates rule definitions, matches rules against
scan entries, and exposes the `RuleRegistry` used by the scanner and planner.

Path globs are compiled with literal separators: `*` stays within one segment
and `**` is required for recursive matching. Broad rules may declare
`match_role = "fallback"`; they remain in evidence but yield the decision to a
trusted primary match, and they can never be default-selected.

Inspection rules (`action = "inspect"`) are non-executable. Their default subtree
scope is inherited by candidates inside retained paths, including explicit child
scans; entry scope supports mixed containers. Shared core protection excludes
retained paths and covering ancestors independently of primary/fallback rules.
Subtree inspectors must be path-only. `exclude_path_globs` refines direct matches;
`parent_marker` requires a regular file observed in the same scan snapshot.

See [cache coverage and sources](../../docs/docs/rules/cache-expansion.md) for the
unreleased developer/AI cache rules and retained-data boundaries. Location
expansion supports bounded child discovery and native current-user ownership;
all new cleanup rules remain review-only with runtime guards.
