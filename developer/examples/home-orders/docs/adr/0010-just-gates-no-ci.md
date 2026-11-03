# Tooling surface: just recipes, no CI files, pinned toolchain

Every former npm script becomes a `just` recipe, and every quality gate — fmt, clippy, `sqlx prepare --check`, nextest, pact, cargo-deny, llvm-cov coverage, SBOM — is a recipe too. Git hooks run via lefthook. CI pipeline files are deliberately absent for now; Tekton pipelines will invoke the same recipes later.

Toolchain: stable Rust, pinned to 1.99.1 in `rust-toolchain.toml`, edition 2024; bumps are deliberate commits. Agent docs stay light: a commands-first `AGENTS.md` (~1 page) and five small skills in `.opencode/skills/` (architecture, development, testing, dependency-management, succinct). The `succinct` skill encodes the house rule: code is the source of truth — comments only when the code isn't self-evident.
