# home-users

A Rust microservice for the home cluster: the `users` reference subgraph from the template (axum + tokio), keeping the template's layered HTTP, typed config, sqlx with a committed offline cache, one error contract, generated OpenAPI and Pact contracts, with `just` recipes as the command surface (ADR-0010). It serves REST under `/v1/users` and, with the `graphql` feature, an Apollo Federation v2 subgraph at `/graphql`.

## Features

- **HTTP** — axum + tower-http: request IDs, structured request logs, response compression, 1 MiB body limit, request timeout, sensitive-header redaction; CORS present but off by default. Every non-2xx is RFC 9457 `application/problem+json` (ADR-0005).
- **OpenAPI 3.1** — generated from the code (DTO annotations + route macros) on non-production startup and by `just openapi`; never hand-edited (ADR-0004).
- **Postgres** — sqlx with compile-time-checked SQL against a committed `.sqlx` offline cache, so builds, tests and image builds never need a live database (ADR-0003). Migrations and seed tooling included.
- **Cache** — Valkey/Redis client, gated by `CACHE_DISABLED`.
- **AMQP** — lapin publisher and a tested consumer module with the frozen always-ack/DLQ semantics (ADR-0006). Dedicated consumer deployments are a per-service recipe (ADR-0002).
- **Logging** — JSON on stdout in every environment, with independent `REQUEST_LOG_LEVEL` and `APP_LOG_LEVEL` filters (ADR-0007).
- **REST** — `GET /v1/users?page=&perPage=` and `GET /v1/users/{id}`, backed by the `users` table.
- **GraphQL (opt-in)** — the `graphql` feature adds an Apollo Federation v2 subgraph at `/graphql`: a `User` entity with `@key(fields: "id")`, `_service`/`_entities`, a repository-backed batched data loader, bearer-token validation against a JWKS before any resolver runs, and self/scope authorization (`@requiresScopes` for Hive plus resolver checks for direct calls). The image builds with the feature enabled (ADR-0011).
- **Lifecycle** — graceful shutdown drains HTTP, cancels consumers and closes pools; `GET /healthz` (liveness) and `GET /readyz` (readiness with a live database check).
- **Testing** — unit/feature tests (nextest + axum-test), pact contract tests on both sides (ADR-0008), acceptance tests against real Postgres/Valkey/RabbitMQ.
- **Containers** — cargo-chef build → distroless `nonroot` runtime (ADR-0009); compose profiles for every flow; multi-arch publishes; a Helm chart in `deploy/home-users`.
- **Supply chain** — cargo-deny (licenses, advisories, bans, sources), CycloneDX SBOM, cargo-auditable release binaries.

## Getting started

Prerequisites: Docker with Compose v2. The `just` recipes additionally need the pinned Rust toolchain (`rust-toolchain.toml`, 1.99.1), `just`, and the cargo tools the recipes invoke (`cargo-nextest`, `sqlx-cli`, `cargo-deny`, `cargo-cyclonedx`, `cargo-auditable`, `cargo-llvm-cov`).

Infrastructure in Docker, app on the host:

```bash
docker compose --profile infra up -d   # Postgres, Valkey, RabbitMQ
just migrate                           # apply migrations
just db-seed                           # optional: reference users (destructive, dev-only)
just dev                               # serve on http://localhost:3000
```

Or the app from mounted source inside Docker, with hot reload and no host toolchain:

```bash
docker compose --profile dev up        # bacon streams rebuilds to the logs
docker compose --profile dev down
```

Both read the committed `.env.development`; the dev container only rewrites the endpoints to its in-network services.

## Configuration

`APP_ENV` (default `development`) picks the committed env file: `.env.development`, `.env.production` or `.env.tests`. The gitignored `.env` (personal overrides) wins over it, and the real process environment wins over both. The typed config (`src/config.rs`) validates everything at startup and fails fast with a readable error.

Logging is JSON on stdout in every environment (ADR-0007), with two independent verbosity filters:

| Variable | Controls |
| --- | --- |
| `REQUEST_LOG_LEVEL` | One line per HTTP request: method, path, status, latency, request ID. `authorization` is never logged. |
| `APP_LOG_LEVEL` | Everything else: business logic, lifecycle, infrastructure. |

Health: `GET /healthz` → 204; `GET /readyz` → 204, or 503 while Postgres is unreachable.

### GraphQL federation (optional)

Build and run with the feature (the default build is unchanged):

```bash
cargo run --features graphql
# or
just test-graphql
```

`GET`/`POST /graphql` then serves an Apollo Federation v2 subgraph. The SDL defines a `User` entity with `@key(fields: "id")`, resolved through a per-request `DataLoader` backed by the users repository; `_service { sdl }` and `_entities` work as a router expects. The queries are `users(page, perPage)`, `user(id)` and `viewer`.

Every GraphQL request requires `Authorization: Bearer <JWT>`. The token is validated (RS256, issuer, expiry, and either `aud` or the Cognito `client_id`) against the configured JWKS before the GraphQL request executes. The caller is available to resolvers as `AuthContext` (`sub`, `email`, `user_id`, `scopes`). Missing or invalid tokens return problem+json **401**; an unreachable JWKS returns problem+json **503**. See `docs/adr/0011-optional-graphql-federation.md`.

Cognito **access** tokens have no `aud`; they carry `client_id` and a space-delimited `scope`. Set `GRAPHQL_JWT_AUDIENCE` to an empty string and `GRAPHQL_JWT_CLIENT_ID` to the API app client id for access tokens. An unset audience keeps the development default; an unset/empty client id disables that check.

| Variable | Default | Purpose |
| --- | --- | --- |
| `GRAPHQL_ENABLED` | `true` when compiled | Set `false` to keep the feature built but not routed |
| `GRAPHQL_JWKS_URL` | `http://localhost:8080/realms/home/protocol/openid-connect/certs` | JWKS endpoint (dev default; set the real IdP) |
| `GRAPHQL_JWT_ISSUER` | `http://localhost:8080/realms/home` | Required `iss` claim |
| `GRAPHQL_JWT_AUDIENCE` | `home-users` | Required `aud` claim; set empty to disable for Cognito access tokens |
| `GRAPHQL_JWT_CLIENT_ID` | unset | Required `client_id` claim (Cognito access tokens) |

Authorization lives next to the data: `users` and `User.email` require the
`users:read` scope (`@requiresScopes` in the SDL, re-checked by the resolvers);
`user(id:)` and `User.email` are visible to the caller's own record or with
`users:read`; `viewer` is always the caller (including its email). The scope
claim is the space-delimited `scope` string Cognito issues. Hive enforces the
same scope at the gateway, so a direct service call and a federated call make
the same field decision.

The endpoint is not part of `openapi.yaml`; REST routes, OpenAPI generation and Pact behavior are unaffected by the feature.

## Testing

| Command | Runs | Needs |
| --- | --- | --- |
| `just test` | Fast suite: unit + feature tests (nextest) and doctests | nothing |
| `just test-graphql` | The fast suite with the optional GraphQL subgraph compiled in | nothing |
| `just test-pact` | Pact consumer generation + provider verification | Postgres on :5432 |
| `just test-acceptance` | Acceptance suite once, against its own compose stack; exits with the suite's status | Docker |
| `just acceptance` | Idle acceptance stack for focused runs | Docker |
| `just coverage` | LCOV report for the fast suite (`lcov.info`) | host toolchain |

`just check` runs the fast gates — formatting, clippy with warnings denied, the sqlx offline-cache check and the fast suite. `just integrity` runs every gate end-to-end and stops at the first failure.

Focused acceptance runs, against the idle stack:

```bash
just acceptance
docker compose --profile acceptance exec acceptance \
  cargo nextest run --features acceptance --profile acceptance -E 'test(cache_round_trips)'
docker compose --profile acceptance down -v
```

## Containers

`just docker-build` produces `home-users:local`: a cargo-chef dependency layer on the pinned Rust slim image, then the binary in a distroless `cc-debian13:nonroot` runtime — no shell, non-root, glibc and CA certificates only (ADR-0009). The build is hermetic: `--locked`, sqlx resolves from the committed offline cache.

Compose profiles cover every flow:

| Profile | Start with | What runs |
| --- | --- | --- |
| `infra` | `docker compose --profile infra up -d` | Postgres, Valkey, RabbitMQ (management UI on :15672) |
| `dev` | `docker compose --profile dev up` | infra + the app from mounted source, hot reload |
| `local` | `docker compose --profile local up --build` | infra + the app built from the Dockerfile |
| `publish` | `IMAGE=registry/app:tag docker compose --profile publish up` | infra + a published image, no build |
| `test` | `just test-acceptance` | infra + a one-shot acceptance runner |
| `acceptance` | `just acceptance` | infra + an idle runner for exec-driven runs |

Every service is profile-gated, so `down` takes the same `--profile` flag as `up` (a bare `docker compose down` is a no-op here). Published ports are remappable through the environment (`API_PORT`, `DATABASE_PORT`, `CACHE_PORT`, `RABBIT_PORT`, `RABBIT_MGMT_PORT`), so this stack can run next to another one.

`just docker-publish` builds and pushes an amd64 + arm64 manifest for `$IMAGE` (default `home-users:latest`); it needs a buildx builder with QEMU and a registry login. `deploy/home-users` is the Helm chart: Deployment, Service, ConfigMap, a Postgres Deployment/PVC/Service, a generated password Secret for `DATABASE_URL`, hardened pods, `/healthz` + `/readyz` probes, the ambient `PeerAuthentication` and a Cilium network policy.

## Documentation

- `AGENTS.md` — commands, rules and crate map for agents working in the repo.
- `GLOSSARY.md` — the vocabulary this template and the services built from it use.
- `docs/PORTING.md` — the Fastify template feature map, including deliberate divergences.
- `docs/adr/` — architecture decisions, one file per decision.
- `docs/research/rust-ecosystem-2026-10.md` — the ecosystem research behind the version picks.

## License

MIT — see `LICENSE`.
