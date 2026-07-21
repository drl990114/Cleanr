# Cleanr Development Guidelines

Guidelines for AI coding agents working in the Cleanr repository.

## Product and Safety Model

Cleanr is a cross-platform, terminal-first disk cleanup tool. It discovers
rebuildable caches and cleanup candidates, explains the evidence, and keeps the
final cleanup decision under human control.

- Treat disk cleanup as a safety-sensitive operation.
- Preserve the evidence-first workflow: scan, explain, review, then confirm.
- Never weaken protected-path checks, overlap handling, trust checks, final
  validation, trash-based cleanup, or restore records for convenience.
- `cleanr analyze` is read-only and AI-friendly. It provides structured local
  evidence; it does not grant an agent cleanup authority by itself.
- An agent may execute only an exact, reviewed plan after the current user
  explicitly delegates that plan. Bind authorization to the plan digest,
  re-scan before execution, and use only system trash plus restore manifests.
- Treat filesystem paths and analysis reports as potentially sensitive local
  data. Do not send them to external services without explicit user approval.

## Tech Stack

- Rust 1.94.1, edition 2024, organized as a Cargo workspace.
- Ratatui and Crossterm for the terminal user interface.
- Serde-based versioned formats for analysis reports, configuration, plugins,
  schemas, and cleanup manifests.
- Docusaurus 3, React 19, and TypeScript for documentation.
- npm launcher packages for distributing platform-specific native binaries.

## Repository Structure

```text
cleanr/
├── crates/             # Rust workspace crates
│   ├── cli/            # CLI entry point and commands
│   ├── core/           # Domain types and cleanup planning
│   ├── fs/             # Filesystem scanning, trash, and restore behavior
│   ├── rules/          # Built-in and plugin cleanup rules
│   ├── tui/            # Ratatui application and views
│   ├── config/         # Configuration loading and policy
│   ├── i18n/           # Runtime translations
│   ├── plugin-api/     # Plugin contracts and schemas
│   └── tasks/          # Task orchestration
├── docs/               # Docusaurus documentation site
├── npm/                # npm launcher and platform package metadata
├── plugins/            # Publishable plugin bundles and index
├── readme/             # Localized README files
├── skills/             # Cross-agent Cleanr skills
└── scripts/            # Maintenance and release automation
```

## Working Principles

### Research before implementation

Before implementing a feature, check whether a maintained open-source library,
existing workspace dependency, or established project pattern already solves
the problem. Compare maintenance, security, portability, binary size, license,
and integration cost. Prefer reuse when it reduces risk and complexity; do not
add a dependency when a small existing abstraction is clearer.

### Evaluate proposals critically

The user's proposed solution may not be the best implementation. Before making
material changes, explain its benefits, drawbacks, and safety implications,
then state the recommended approach and why. Preserve the user's underlying
goal while challenging assumptions that would increase risk or complexity.

### Keep changes focused

- Read neighboring modules, crate READMEs, and existing tests before editing.
- Preserve unrelated user changes in a dirty worktree.
- Keep platform behavior aligned across macOS, Linux, and Windows.
- Prefer small, explicit changes over broad refactors.
- Update user documentation when commands, defaults, schemas, safety behavior,
  or supported platforms change.
- Keep the root, English, and Simplified Chinese READMEs synchronized when
  changing shared product messaging.

## Rust Conventions

- Respect crate boundaries; put shared domain behavior in the owning library
  crate rather than the CLI or TUI.
- Avoid `unsafe` unless no safe design is practical and the invariant is
  documented and tested.
- Return structured errors with context instead of panicking in user flows.
- Keep serialized contracts backward-aware and update schema versions when a
  compatibility-breaking format change is intentional.
- Add or update translations for user-visible strings.
- Use deterministic ordering for plans, reports, and generated metadata.

## Documentation and Skills

- English documentation lives in `docs/docs/`; Simplified Chinese lives in
  `docs/i18n/zh-Hans/docusaurus-plugin-content-docs/current/`.
- Keep examples executable and do not document planned behavior as released.
- Keep AI-facing instructions concise, imperative, and explicit about safety.
- Validate a modified skill with the `skill-creator` validator.
- The Cleanr skill may install the CLI globally when it is missing, but it must
  never reinstall or upgrade an existing CLI unless the user asks.

## Quality Checks

Run only checks relevant to the changed files. Do not use a build as
verification.

For Rust formatting and linting:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

For documentation TypeScript:

```bash
cd docs
pnpm typecheck
```

For Skill metadata and structure:

```bash
uv run --with pyyaml python \
  "$HOME/.codex/skills/.system/skill-creator/scripts/quick_validate.py" \
  skills/cleanr-review-disk-cleanup
```

Do not run `cargo build`, `pnpm build`, or a full test suite unless the user
explicitly requests it. Always run `git diff --check` before handing off.
