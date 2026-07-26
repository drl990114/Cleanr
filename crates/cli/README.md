# cleanr-cli

Cleanr command-line interface.

This crate provides the `cleanr` binary, the main entry point for the cleanr
suite. It wires configuration, internationalization, and the terminal user
interface together. Non-interactive cleanup requires an exact reviewed plan,
its SHA-256, and explicit user authorization; execution remains trash-based.
`plan` and `dry-run` accept exact candidate-path selection overrides so a local
agent can encode choices the user made during evidence review without editing
the plan or bypassing candidate and safety checks.
