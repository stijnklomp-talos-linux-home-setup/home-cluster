---
name: dependency-management
description: Use when adding, upgrading or removing a crate, when a cargo-deny check fails (`just licenses`), or when deciding how to pin a version.
---

# Dependencies

- Before adding a crate, check `docs/research/rust-ecosystem-2026-10.md`: it records the evaluated picks per concern and the crates to avoid. Reuse something already evaluated before reaching for something new.
- Toolchain: Rust 1.99.1, edition 2024 — pinned in `rust-toolchain.toml` (MSRV `rust-version = "1.99"` in `Cargo.toml`); the Dockerfile and compose images pin the same `rust:1.99.1-slim-trixie` tag. Bumps are deliberate commits, not side effects.
- `Cargo.lock` is committed and authoritative; the image builds `--locked`. Commit lock changes together with the manifest change.
- `just licenses` runs cargo-deny (licenses, advisories, bans, sources — config in `deny.toml`). Widening the license allow-list needs a deliberate, reasoned `deny.toml` change; advisories are ignored per-ID with a reason, never per-crate.
- `just sbom` produces the CycloneDX SBOM and an auditable release binary. Supply-chain gates follow ADR-0010: `just` recipes, no CI files.
- After any dependency change: `just check` and `just licenses`.
