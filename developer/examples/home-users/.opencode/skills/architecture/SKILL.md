---
name: architecture
description: Use when adding a route, business logic, database query, or infrastructure client, or when deciding which layer a change belongs to.
---

# Architecture: a layered service app

One binary (ADR-0002): `src/main.rs` wires and serves; `src/lib.rs` holds the modules.

## Request path

`routes/` (axum registration + `#[utoipa::path]`) → `controllers/` (extract + validate, map errors, shape responses) → `services/` (business logic; dependencies behind traits so tests use hand-written fakes) → `repositories/` (compile-time-checked SQL) → Postgres.

`models/` holds the DTOs: one struct per shape carrying `serde` + `utoipa::ToSchema` + `garde::Validate` (ADR-0004).

## Supporting modules

- `server.rs` — router assembly and the middleware pipeline (security headers, request-id, trace, sensitive headers, compression, body limit, timeout; CORS is present but off).
- `config.rs` — typed env config, loaded and validated at startup, fail fast. Add a setting here, not with scattered `env::var` reads.
- `logging.rs` — the two independent stdout JSON filters (ADR-0007). `problem.rs` — RFC 9457 errors (ADR-0005): don't invent another error shape. `health.rs` — `/healthz`, `/readyz`. `shutdown.rs` — cleanup registry: register every long-lived client.
- `infrastructure/` — cache client. `amqp.rs` — publisher; `consumer.rs` — consume loop and the dedicated-worker recipe (module docs).
- `openapi.rs` + `src/bin/openapi.rs` — the document generated from the routes; never hand-edited.

## Rules

- Business logic in services; controllers stay thin; SQL in repositories.
- A new infrastructure client: its own module owning the connection, plus cleanup registration in `main.rs`.
- A new route is registered in `routes/` once — it appears in the OpenAPI document from the same source.
- Where the vocabulary is unclear, check `GLOSSARY.md`. Where a decision looks wrong, read the ADR before diverging — flag conflicts instead of silently overriding.
