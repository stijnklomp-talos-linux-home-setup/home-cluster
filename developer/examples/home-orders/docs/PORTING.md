# Porting: Fastify template → Rust

Feature inventory of the Fastify template (`fastify-template/`) plus real-world patterns from `fragment-composer/`, and how each feature lands in this Rust template. Decisions are recorded in `docs/adr/`; version/ecosystem facts in `docs/research/rust-ecosystem-2026-10.md`.

Ground rules:

- **Fresh start.** No compatibility with existing Fastify services is needed.
- TypeScript/Fastify-specific mechanics are replaced with idiomatic Rust; conventions (layering, naming, lifecycle, AMQP behavior, DX shape) are kept.
- Code is the source of truth: nothing hand-maintained that code can generate; docs stay lean.

## Process model

- **One binary — the service app.** `DEPLOYMENT_MODE` (api/worker) was a Node workaround and is gone (ADR-0002).
- AMQP consumer/DLQ code ships as a tested library module. A service that needs a dedicated consumer deployment adds a small, documented `src/bin/worker.rs` (or wires the consumer in-process).
- The demo slice (`notes` endpoints, seed tooling, their tests) carries removal-marked header comments.

## Features by area

### HTTP & middleware → axum + tower-http

| Fastify (TS) | Rust |
| --- | --- |
| Fastify 5 route objects | axum 0.8 routers, `{param}` path syntax |
| `@fastify/helmet` security headers | **security-headers layer**, every response: `x-content-type-options: nosniff`, `x-frame-options: SAMEORIGIN`, `referrer-policy: no-referrer`, `x-dns-prefetch-control: off`, `x-download-options: noopen`, `x-permitted-cross-domain-policies: none`; CSP and HSTS deliberately omitted (app-specific / needs TLS) |
| CORS file (fully commented out) | `CorsLayer` present, off by default |
| `@fastify/sensible` | problem+json error layer (ADR-0005) |
| `hyperid` request IDs | request-id layer (uuid) |
| SIGTERM handling (AMQP only) | real graceful shutdown: HTTP drain, consumer cancel, pools close |
| `GET /healthz` → 204 | same |
| `GET /readyz` → 204 / 503 with fresh DB connect check | same |

### Config & environment

- Scattered `process.env` reads → typed config struct (config-rs + dotenvy), validated at startup, fail fast.
- `.env.development` / `.env.production` / `.env.tests` kept.
- Env contract:
  - **Kept**: `API_PORT`, `DATABASE_*`, `CACHE_*`, `RABBIT_*`, `WORKER_QUEUE`, `WORKER_EXCHANGE`, `WORKER_BINDING_KEY`, `REQUEST_LOG_LEVEL`.
  - **Renamed**: `CUSTOM_LOG_LEVEL` → `APP_LOG_LEVEL`; `NODE_ENV` → `APP_ENV`.
  - **Dropped**: `DEPLOYMENT_MODE`, `SHOW_LOGS`, `ENV`, `HOST_UID`/`HOST_GID` (compose specifics kept as-is where useful).
  - **New**: `WORKER_PREFETCH` (default 1), `OTEL_EXPORTER_OTLP_ENDPOINT` (reserved hook — the exporter is deliberately deferred until OTel Rust stabilizes; see the map's Fog and ADR-0007), `IMAGE` (compose `publish` profile).
  - **Fix**: `RABBIT_PORT` default corrected to 5672 (the TS code defaulted to 5671 while every env file used 5672).

### Logging

- pino → `tracing` + `tracing-subscriber`: JSON on stdout, identical in every environment; no in-app shipping (ADR-0007).
- Two independent per-layer filters: `REQUEST_LOG_LEVEL` (request logs) and `APP_LOG_LEVEL` (all other logs).
- pino serializers/redaction ported: authorization headers never logged; request log lines carry method, path, status, latency, request ID.
- `pino-pretty` dev transport dropped deliberately — one format everywhere.

### Validation & OpenAPI

- TypeBox + `@fastify/swagger` (write `openapi.yaml` on non-production start) → one struct per DTO carrying `serde` + `utoipa::ToSchema` + `garde::Validate` (axum-valid extractors); OpenAPI 3.1 **always generated from code** (non-production startup write + `just openapi`); never hand-edited (ADR-0004).
- No Swagger UI (parity — the TS template never had one).

### Error handling

- Three competing shapes (`{error}`, `{message}`, `{error: "Service Unavailable"}`) → one: RFC 9457 `application/problem+json` for every non-2xx, one shared `Problem` type, validation details as an extension field (ADR-0005).

### Database (Prisma → sqlx)

- Prisma client + repositories → sqlx 0.9 + repositories layer; compile-time-checked SQL; committed `.sqlx` offline cache (`SQLX_OFFLINE=true`) so builds/tests need no live DB; `cargo sqlx prepare --check` in `just check` (ADR-0003).
- Prisma migrations → sqlx-cli migrations with the service's own tracking table (`just migrate`, `just db-reset`).
- **New**: `just db-seed` populates the dev database with mock data (dev-only, destructive; marked as demo tooling).
- Prisma Studio → not replaced (no equivalent shipped).
- Repository pattern, model naming, and query style carry over; Prisma-generated entities are not missed (no generated layer to keep in sync).

### Cache (node-redis → redis-rs)

- `cacheClient` singleton semantics, `CACHE_*` envs, `CACHE_DISABLED` gating → redis-rs with the same shape.

### AMQP (amqplib wrapper → lapin)

- Wrapper behavior **frozen** (ADR-0006): exchanges `main` (alternate `nonRouted`, 24h TTL) and `deadLetter`; `task_queue` bound `task.#` (1h TTL, DLX `deadLetter`); declare-on-both-start; publish declares channel + exchange; manual ack; failure → republish to `<queue>.failed` with error metadata, then always ack; no requeue.
- Transport upgrades (deliberate): automatic reconnection, awaited publisher confirms, `WORKER_PREFETCH` (default 1; was effectively unlimited).
- Worker stub (`DEPLOYMENT_MODE=worker` + `processTask`) → consumer module with the same always-ack/DLQ behavior, tested; demo task handler kept as the example.

### Testing (bun test → nextest)

- Unit tests: module mocking + `bun-mock-prisma` → in-crate tests with hand-written fakes behind traits (no module mocking in Rust).
- Feature tests: `app.inject` → axum-test, in-process.
- Acceptance tests: empty scaffolding in TS → real full-stack tests in `tests/acceptance/` (feature-gated), run through Docker Compose profiles (`test` = once and exit; `acceptance` = idle stack for manual runs/debugging); the real-broker integration tests (`tests/amqp.rs`, `tests/consumer.rs`) run in the same `acceptance` nextest profile, so the default profile (`just test` = `cargo nextest run` + `cargo test --doc`) needs no infrastructure.
- Coverage: bun threshold config → `just coverage` via cargo-llvm-cov (no threshold gate shipped — no CI yet).
- Test fixtures and helpers: ported where useful.

### Pact (composer addition → first-class)

- Provider verification only, unreachable from scripts, no provider states → both sides (ADR-0008): consumer tests generate pacts (`pact_consumer`), provider verification in `tests/pact/` (`pact_verifier`) with a provider-state handler map; a working example of each side ships; `just test-pact`; no broker (documented publish hook).

### Docker & Compose

- Bun multi-stage Dockerfile → cargo-chef + BuildKit cache → distroless `cc-debian13:nonroot`; one image for the service app; multi-arch (amd64 + arm64) manifest via `just docker-publish` (ADR-0009).
- Compose profiles: `infra` (Postgres/Valkey/RabbitMQ only), `dev` (source-mounted hot reload), `local` (build from Dockerfile), `publish` (run a **published** image from the registry — no build), `test` (stack + acceptance once, exit), `acceptance` (stack idle; exec in to run all/one test or debug).
- Service keys are `db` / `cache` / `rabbitmq` — a divergence from ticket #4's `postgres` / `valkey` / `rabbitmq` letter, chosen to mirror the Fastify template so the committed `.env.development` values (`CACHE_HOST=cache`, `RABBIT_HOST=rabbitmq`) and the `dev` service's in-network `DATABASE_URL` (`db`) stay coherent.
- `scripts/` (integrity, migrate, check-licenses, sbom) → all become `just` recipes.

### Kubernetes

- Template ships plain manifests: `Deployment` / `Service` / `ConfigMap` with liveness (`/healthz`) and readiness (`/readyz`) probes; non-root distroless image fits the cluster defaults. No CronJob example (composer's cron jobs remain service-specific).

### DX & repo tooling

- husky + lint-staged → lefthook; ESLint + Prettier → rustfmt + clippy (workspace lints; `-D warnings` enforced by `just lint`); TypeDoc → `cargo doc` (no extra config); `minimumReleaseAge` install policy → cargo-deny + pinned lockfile.
- Former npm scripts → `just` recipes 1:1 (`check`, `build`, `dev`, `fmt`, `lint`, `test`, `test-feature`, `test-pact`, `test-acceptance`, `coverage`, `migrate`, `db-reset`, `db-seed`, `openapi`, `licenses`, `sbom`, `integrity`, …).
- License checking (custom script) → cargo-deny; SBOM (custom script) → cargo-cyclonedx + cargo-auditable.
- No CI pipeline files yet — Tekton calls the same recipes later (ADR-0010).
- `.editorconfig` kept/adapted; `bunfig` aliases and `jsonsort` tooling dropped (not applicable).
- Per-directory `README.md` files dropped in favor of a self-evident layout + AGENTS.md (lean docs).

### Repo conventions & agent docs

- Proposed layout mirrors the TS layers: `src/{main.rs, lib.rs, api/, routes/, controllers/, services/, repositories/, models/, infrastructure/, worker/, middleware/, config/, common/}`; tests in `tests/{feature, pact, acceptance}`.
- `.opencode/skills/`: five light skills — architecture, development, testing, dependency-management, **succinct** (code is the source of truth; comments only when the code isn't self-evident; testing kept minimal by design).
- `AGENTS.md`: slim, commands-first (~1 page), written once the commands exist.
- Demo slice: `GET/POST /v1/notes`, seed tooling, their tests — all header-marked "example, remove when building your service".

## Deliberate divergences from the Fastify template

1. One binary; no `DEPLOYMENT_MODE`.
2. `CUSTOM_LOG_LEVEL` → `APP_LOG_LEVEL`; `NODE_ENV` → `APP_ENV`.
3. One error contract (RFC 9457) instead of three shapes.
4. sqlx instead of Prisma (fresh start; no interop needed).
5. AMQP: reconnection + confirms + configurable prefetch added; all observable semantics unchanged.
6. Acceptance tests are real (TS shipped scaffolding only).
7. No CI files; every gate is a `just` recipe.
8. Per-directory READMEs dropped.
9. Seed tooling added.
10. Multi-arch manifest builds (`just docker-publish`) and a `publish` compose profile.

## Suggested build order (next phase)

1. Skeleton: Cargo.toml, `rust-toolchain.toml`, lints, rustfmt, justfile, lefthook, env files, ignore files.
2. Core: config, logging, problem+json errors, health.
3. Data: sqlx pool, migrations, seed, notes demo slice.
4. HTTP: routes/controllers/services/repositories, layers, OpenAPI generation.
5. Infrastructure: cache client, AMQP publisher + consumer module, with tests.
6. Tests: unit/feature/acceptance, pact examples.
7. Dockerfile + compose profiles; k8s manifests.
8. Skills, AGENTS.md, README.
