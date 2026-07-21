# cleanr-rules

Rule engine and registry for cleanr.

This crate parses rule packs, validates rule definitions, matches rules against
scan entries, and exposes the `RuleRegistry` used by the scanner and planner.

Path globs are compiled with literal separators: `*` stays within one segment
and `**` is required for recursive matching. Broad rules may declare
`match_role = "fallback"`; they remain in evidence but yield the decision to a
trusted primary match, and they can never be default-selected.
