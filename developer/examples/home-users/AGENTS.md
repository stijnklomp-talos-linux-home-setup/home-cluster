# AGENTS.md

Rust microservice for the home cluster (the `users` reference subgraph): REST + optional Federated GraphQL (feature `graphql`) over Postgres. Vocabulary: `GLOSSARY.md`. Decisions: `docs/adr/`. Fastify → Rust feature map: `docs/PORTING.md`.

## Commands

Everything runs through `just` (ADR-0010: the recipes are the gates; there are no CI files). `just` alone lists them.

| Recipe | Purpose |
| --- | --- |
| `just check` | The gate: `fmt-check`, `lint`, `sqlx-check`, `test`. Needs Postgres on localhost:5432. |
| `just test` | Fast suite: unit + feature tests (nextest) + doctests. No infrastructure. |
| `just test-graphql` | The fast suite with the optional GraphQL subgraph (feature `graphql`, ADR-0011). |
| `just fmt` / `just fmt-check` | Format the crate / check formatting. |
| `just lint` | Clippy with `-D warnings`. |
| `just build` / `just dev` | Build / run the service app. |
| `just test-pact` | Pact consumer generation + provider verification. Needs Postgres. |
| `just test-acceptance` | Acceptance suite once against its own compose stack; exits with the suite's status. |
| `just acceptance` | Idle acceptance stack for `docker compose --profile acceptance exec ...` runs. |
| `just coverage` | LCOV report for the fast suite (`lcov.info`, gitignored). |
| `just migrate` / `just db-reset` / `just db-seed` | Migrations / drop-and-recreate / demo data. Need Postgres; reset and seed are destructive. |
| `just sqlx-prepare` / `just sqlx-check` | Refresh / verify the committed `.sqlx` offline cache. |
| `just openapi` | Regenerate the committed `openapi.yaml` from the code. |
| `just licenses` / `just sbom` | cargo-deny checks / CycloneDX SBOM + auditable release build. |
| `just integrity` | Every gate end-to-end, aborting at the first failure. Heavy; needs infrastructure. |
| `just docker-build` / `just docker-publish` | Local image / multi-arch manifest build-and-push. |

## Rules

- **sqlx offline**: `.sqlx/` is committed and `.cargo/config.toml` sets `SQLX_OFFLINE=true`, so builds, tests and image builds never need a live database. After changing queries or migrations: `just migrate && just sqlx-prepare`, commit `.sqlx/` in the same change. `just check` runs `cargo sqlx prepare --check`, so a stale cache fails the gate. Never hand-edit `.sqlx/`.
- **OpenAPI is generated** (`just openapi`, ADR-0004): change the routes/DTOs and regenerate; never hand-edit `openapi.yaml`.
- **GraphQL** (ADR-0011): the `graphql` feature is required for `/graphql`; the Dockerfile builds with it. Every GraphQL request needs a valid JWT (`GRAPHQL_*`); **REST `/v1/*` stays unauthenticated and unowned by design** (the in-cluster contract), so it must not be exposed as the authenticated path. Cognito access tokens have no `aud`: set `GRAPHQL_JWT_AUDIENCE=""` and `GRAPHQL_JWT_CLIENT_ID`. Authorization is self-or-`users:read` for `user(id:)`, and `users:read` for the list; `User.email` has **no** `@requiresScopes` (Hive cannot express "self or scope"), so its resolver is the single guard and self-email works without `users:read`. Self-by-email requires `email_verified: true` in the token. `@requiresScopes` on the list is the gateway's coarse check; resolvers are the direct-call check. The `User` entity is backed by the users repository through `UserLoader`; keep entity resolution batched. `User.initials` is a computed field derived from `name` (no column, no migration) — it exists to prove a schema-only change can reach the gateway without a service rebuild (ticket #18).
- **Deployment** lives in `deploy/home-users` (Helm): namespace `home-users` is ambient + Pod Security `restricted`; the chart ships Postgres, the generated `DATABASE_URL` Secret, a post-install/post-upgrade migration Job (`/app/migrate`), `PeerAuthentication` STRICT and the Cilium policy (same namespace + the `home-graphql` gateway; host/health probes). Image reference is the local registry (`192.168.1.210:5000/home-users`).
- **One binary** (ADR-0002): the app starts no consumers. A service that needs a dedicated consumer adds `src/bin/worker.rs` — the recipe and the frozen ack/DLQ semantics are in the `src/consumer.rs` module docs.
- **Run `just check` before declaring work done.**

## Crate map

- `src/main.rs` — wiring only: config → logging → clients → router → serve → cleanup.
- `src/lib.rs` — `routes` → `controllers` → `services` → `repositories` (+ `models` DTOs), `infrastructure/` (cache), `amqp.rs` + `consumer.rs`, `config.rs`, `logging.rs`, `problem.rs` (RFC 9457), `health.rs`, `server.rs`, `shutdown.rs`, `openapi.rs`, and `graphql/` (opt-in federation subgraph, feature `graphql`, ADR-0011).
- `src/bin/` — `openapi` (writes `openapi.yaml`), `migrate` (Helm migration Job), `db_seed` (reference users).
- `tests/` — feature tests in `*.rs`, contract tests in `pact/`, the feature-gated acceptance suite in `acceptance/`.
- `migrations/`, `docker-compose.yml` (profiles: infra/dev/local/publish/test/acceptance), `deploy/home-users` (Helm chart), `pacts/` (checked-in contracts), `examples/`.

## Pitfalls

- nextest has no doctest support; `just test` chains `cargo test --doc` for them — don't drop it.
- The acceptance suite needs `--features acceptance` and the compose stack; a plain `cargo nextest run` skips both it and the real-broker tests by design (`.config/nextest.toml`).
- `docker compose down` needs the same `--profile` flag as `up` — every service is profile-gated, so a bare `down` is a no-op.

## Agent docs

- Issue tracker: GitHub issues via `gh` — `docs/agents/issue-tracker.md`.
- Triage labels — `docs/agents/triage-labels.md`.
- Domain docs: single context — `GLOSSARY.md` + `docs/adr/` — `docs/agents/domain.md`.
- Skills: `.opencode/skills/` (architecture, development, testing, dependency-management, succinct).
