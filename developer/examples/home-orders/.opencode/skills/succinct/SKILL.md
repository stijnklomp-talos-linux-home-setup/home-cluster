---
name: succinct
description: Use when about to add comments, doc comments, or prose docs, when deciding where a fact belongs (code vs. docs), or when existing docs and code disagree.
---

# Succinct: the code is the source of truth

Code, tests and generated artifacts are the source of truth. Prose is for what the code cannot say.

## Rules

- **Comments explain why, not what.** Delete comments that restate the next line. Keep them for non-obvious constraints, frozen semantics, and pointers to the ADR that explains the decision.
- **Docs point; they don't duplicate.** `See src/consumer.rs` or `see ADR-0006` beats restating behavior that will drift.
- **Never hand-edit generated artifacts** (`openapi.yaml`, `.sqlx/`, `sbom.json`): change the source and regenerate (`just openapi`, `just sqlx-prepare`).
- **Demo-slice markers are the deliberate comment exception**: `example — remove when building your service` exists to tell the reader what to delete — keep it until the slice goes.
- When prose and code disagree, the code wins: fix or delete the prose.

## Smells

- A comment that narrates syntax (`// loop over notes`).
- The same fact stated in AGENTS.md, README.md, an ADR and a module doc.
- A paragraph explaining a mechanism the reader can read in the source, but no sentence explaining why it is that way.
