---
name: development
description: Use when running the app locally, changing the schema or a query, or regenerating openapi.yaml or the .sqlx offline cache.
---

# Development loop

Stack up: `docker compose --profile infra up -d` gives Postgres (:5432), Valkey (:6379) and RabbitMQ (:5672, management :15672) with the `.env.development` defaults. `just dev` runs the app on :3000; `docker compose --profile dev up` runs it from mounted source with hot reload and no local Rust install.

## Changing the database

1. Add a migration under `migrations/` (sqlx-cli naming), or change the repository query.
2. `just migrate` applies it; `just db-reset` rebuilds from scratch (destructive).
3. `just sqlx-prepare` refreshes the committed `.sqlx/` cache — commit it in the same change; never hand-edit it.
4. `just db-seed` loads demo notes (destructive, dev-only; refuses `APP_ENV=production`).

Builds and tests are offline: `.cargo/config.toml` sets `SQLX_OFFLINE=true`, so sqlx macros resolve against `.sqlx/`, not a live database.

## Changing the API

DTOs and `#[utoipa::path]` annotations are the source; run `just openapi` and commit the regenerated `openapi.yaml` (never hand-edit it).

## Before finishing

`just check` runs fmt, clippy `-D warnings`, the sqlx cache check and the fast suite (it needs Postgres on :5432). `just integrity` runs every gate end-to-end. Quality gates are `just` recipes by design — CI files don't exist; add a recipe instead (ADR-0010).
