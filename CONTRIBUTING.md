# Contributing to Wireglyph

Wireglyph is an experimental local-first repository map. Small, evidence-backed
changes are welcome.

## Before opening a change

- Open an issue for a new language, a graph-contract change, or a substantial UI
  change before investing in implementation.
- Keep extracted relationships tied to repository-relative file and line
  evidence.
- Never label static analysis as observed runtime behavior.
- Use synthetic fixtures or Wireglyph itself for tests and screenshots. Do not
  commit proprietary repositories, credentials, source snippets, or third-party
  design-reference collections.

## Local validation

Wireglyph requires Rust 1.88 or newer.

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
git diff --check
```

Visual changes should include a regenerated proof at 100x30 or 140x40 cells and
must preserve reduced-motion, monochrome, compact-terminal, and terminal-cleanup
behavior.

## Pull requests

Explain what changed, why it is truthful, the explicit non-goals, and the exact
validation performed. Keep generated or dependency-only churn separate from
behavior changes whenever practical.

By contributing, you agree that your contribution is licensed under the same
MIT OR Apache-2.0 terms as Wireglyph.
