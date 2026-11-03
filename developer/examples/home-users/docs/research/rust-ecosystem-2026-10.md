# Rust ecosystem research — agent-friendly microservice template (October 2026)

Scope: porting the Fastify 5 + TypeScript template (Bun, TypeBox→OpenAPI 3.1, Prisma 7/Postgres, amqplib RabbitMQ `main`/`deadLetter` topic exchanges + `task_queue` bound `task.#`, node-redis/Valkey, pino with two log levels, jose JWT, `DEPLOYMENT_MODE=api|worker`, Kubernetes, Pact provider verification against checked-in pacts) to Rust, with ~90% of development done by AI agents.

Method: every version, date and maintenance claim below was pulled live on **2026-10-15** from primary sources: the crates.io API (max_stable_version, version `created_at`, recent downloads, repository field), GitHub release feeds (`releases.atom`) and repository source/README/changelogs, plus official docs (docs.rs, rust-lang.org, Prisma docs, agents.md). Anything I could not verify live is marked **unverified**. Where sources conflict, it is called out.

**Headline changes since 2024–2025 that overturn older advice**

- **prisma-client-rust is dead.** Officially unmaintained since 2025-03-12, repo archived, last release 0.6.11 (2024-01-06) supporting Prisma 4.8. Prisma 7 (2025-11-19) made the Prisma Client **Rust-free by default** (TypeScript query compiler + driver adapters), so the crate can never track Prisma 7 anyway. Any 2023–24 advice to "keep using the Prisma client from Rust" is obsolete.
- **sqlx 0.9 moved orgs and grew a config file.** The repo moved from `launchbadge/sqlx` to `transact-rs/sqlx`; `Cargo.lock` removed from the repo (`cargo install --locked sqlx-cli` no longer works); new `sqlx.toml` lets you rename `DATABASE_URL`, override the `_sqlx_migrations` table (explicitly for multiple crates sharing one DB), and pin type mappings. MSRV 1.94.
- **lapin now has built-in connection recovery** (`enable_auto_recover()`), which the TS stack never had.
- **axum is still 0.8.x** (0.8.9, Apr 2026); the next breaking version (~0.9) is in development but unreleased as of today.
- **OpenTelemetry Rust is still 0.x** (0.33.1) with **traces in Beta** — no 1.0 yet.
- **`async-std` is deprecated** (the SeaORM 2.0 migration guide states it; sqlx keeps the feature flags but the ecosystem default is tokio).
- **Rust 1.99.1** is current stable (released today); **edition 2024** is the current edition; an edition for 2027 is being discussed but *not committed* (sources conflict — see §11).

---

## 1. Web framework

**Candidates and health**

- **axum 0.8.9** (2026-04-14) — tokio-rs; ~133M recent downloads; release cadence steady (0.8.8 Dec 2025, 0.8.9 Apr 2026); MSRV 1.80. The "Unreleased" changelog section contains the next breaking release (~0.9): `tower-log` off by default, `axum::serve` changes (header read timeout, executor hooks, connection limits, max connection age), Router fallback/merge changes. https://crates.io/crates/axum · https://github.com/tokio-rs/axum/blob/main/axum/CHANGELOG.md
- **actix-web 4.17.0** (2026-10-15) — actix org; ~11.7M recent downloads; very active. Actor-influenced, its own middleware system (not tower-compatible); slight raw-throughput edge, irrelevant for this workload. https://crates.io/crates/actix-web
- **poem 3.1.12** (2025-07-28) — slowing release cadence; smaller ecosystem. **Rocket 0.5.1** (2024-05-23) — effectively stalled (two years without a release; long-promised async work shipped in 0.5 but momentum moved on). Both are not recommended for a new template in 2026.
- **hyper 1.12.0** (2026-10-06) + **hyper-util 0.1.21** (2026-09-24) — hyperium; extremely active (hyper is the transport under axum and actix-web's tokio path). Raw hyper+tower is viable but strictly more boilerplate than axum, which is itself a thin layer over hyper/tower.
- **tower 0.5.3** (2026-01-12), **tower-http 0.7.1** (2026-08-31; 0.7.0 Jun 2026), **tokio 1.53.3** (2026-10-13) — all tokio-rs/tower-rs; the de-facto middleware substrate.

**2026 consensus**: independent 2025–2026 comparisons and migration guides converge on axum as the default for new Rust services (tower ecosystem, tokio alignment, growing adoption); actix-web remains the credible alternative for existing codebases or extreme throughput. The old `:id`/`*rest` path syntax was replaced by `{id}`/`{*rest}` in **axum 0.8.0** (matchit 0.8; old syntax now panics) — this aligns routes with OpenAPI templating, useful for the utoipa generation later. matchit 0.9.2 (2026-04-08) added captures with static prefixes/suffixes.

**Middleware mapping (all tower-http 0.7.1)** — verified module list from the repo:
request IDs → `request_id` (`SetRequestIdLayer` + `PropagateRequestIdLayer` + `MakeRequestUuid`); tracing/request logs → `trace::TraceLayer`; timeouts → `timeout::TimeoutLayer` (plus `TimeoutLayer` from tower for services); body limits → `limit::RequestBodyLimitLayer`; CORS → `cors::CorsLayer`; compression → `compression::CompressionLayer` / `DecompressionLayer`; header redaction → `sensitive_headers::SetSensitiveRequestHeadersLayer`; metrics → new `metrics` module (0.7). https://docs.rs/tower-http

**Graceful shutdown for Kubernetes** — the canonical axum example uses `tokio::signal` (SIGINT + SIGTERM via `signal::unix::SignalKind::terminate()`) and `axum::serve(listener, app).with_graceful_shutdown(shutdown_signal())`, with a `tower_http::timeout::TimeoutLayer` bounding in-flight requests so shutdown terminates. This is exactly what the Fastify `cleanupOnExit` SIGTERM handlers emulate today. https://github.com/tokio-rs/axum/blob/main/examples/graceful-shutdown/src/main.rs

**Agent-friendliness**: axum docs are excellent, the examples directory is large and canonical (graceful-shutdown, request-id, tracing, jwt, validator, prometheus-metrics, testing, sqlx-postgres, todos…), and errors are typed and predictable. actix-web's extra concepts (worker closure, `web::Data`, non-tower middleware) cost agent iterations for no benefit here.

→ **Recommendation**: **axum 0.8.9 + tower-http 0.7.1 on tokio 1.53**, with the standard layer stack (request-id → trace → sensitive-headers → cors → compression → limit → timeout), `{param}` route syntax, and `tokio::signal` + `with_graceful_shutdown` for K8s. Don't wait for 0.9; the eventual migration is small and mechanical.

---

## 2. Database access + migrations

**Candidates and health**

- **sqlx 0.9.0** (2026-05-06/21; repo now `transact-rs/sqlx`) — ~46M recent downloads; very active; MSRV 1.94. Async-first, pool built in, SQL-first (no DSL), compile-time checked queries via macros, first-class Postgres types. https://crates.io/crates/sqlx
- **SeaORM 2.0.4** (2026-09-27; 2.0.0 on 2026-07-20 after 43 RCs) — SeaQL; active; big 2.0 release (dense entity format, nested ActiveModel, SeaQuery 1.0, sqlx 0.9, RBAC). Entity codegen needs a live DB (`sea-orm-cli generate entity`); migration table is `seaql_migrations` and **configurable** via `MigratorTrait::migration_table_name()`. Raw SQL escape hatches: `execute_raw`/`query_*_raw`. https://crates.io/crates/sea-orm · https://github.com/SeaQL/sea-orm/releases/tag/2.0.0
- **Diesel 2.3.14** (2026-10-07) + **diesel-async 0.9.2** (2026-06-19) — active; sync-first, async layer separate; type-level compile checking works offline, but `print-schema` needs a live DB; migration table is hardcoded `__diesel_schema_migrations` (verified in source — no custom table name). Heavier type gymnastics and worse error messages for agents.
- **Toasty 0.11.0** (2026-09-25) — tokio-rs org, fast cadence (0.10 Aug 2026 → 0.11 Sep 2026), but 0.x and small adoption (~52k recent downloads); own model DSL; no proven story for sharing migrations with Prisma. Watch, don't adopt.
- **prisma-client-rust** — **UNMAINTAINED/dead** (see headline). Do not use.
- **refinery 0.10.0** (2026-10-02) — active; embedded/run-at-startup migrations tracked in `refinery_schema_history`; useful only if you specifically need Rust-owned idempotent migration execution.

**The zero-live-DB story (critical for CI/agents)** — sqlx: **offline mode is now always enabled**; run `cargo sqlx prepare` (or `--workspace`) to write the `.sqlx` query cache, **commit it**, and set `SQLX_OFFLINE=true` (e.g. in `.env`) so a stray `DATABASE_URL` never forces a DB connection at build time; CI runs `cargo sqlx prepare --check` to fail when the cache is stale. Queries are still compile-time checked — against the cached metadata instead of a live DB. Also from the sqlx README: `[profile.dev.package.sqlx-macros] opt-level = 3` speeds up incremental builds of the macro-heavy code. https://github.com/launchbadge/sqlx/blob/main/sqlx-cli/README.md#enable-building-in-offline-mode-with-query

**Compile-time checking per tool**: sqlx — DB-verified via macros + `.sqlx` offline cache (best fit for agents); SeaORM — codegen from a live DB (or hand-written entities), no per-query compile checking; Diesel — type-level DSL checking offline, but schema must be generated from a live DB; Toasty — own DSL.

**Postgres types**: sqlx — `Vec<T>` for arrays (`text[]`), `Json<T>`/`serde_json::Value` via the `json` feature, `chrono`/`time`/`uuid` features all present in 0.9 (feature list verified). Prisma's `DateTime` maps to `timestamp(3)` in Postgres → use `chrono::NaiveDateTime` unless the column is `timestamptz` (then `DateTime<Utc>`). SeaORM 2 handles PG arrays and JSON well (array codegen fixes were in the 2.0 train).

**Migration interop with Prisma (same Postgres DB, `_prisma_migrations` owned by Prisma)** — Prisma 7 docs confirm the `_prisma_migrations` table with checksums; `migrate deploy` only replays migration files, and the shadow DB is only needed for `migrate dev`. Coexistence rules that work:

1. **Every tool tracks migrations in its own table** — `_prisma_migrations` (Prisma), `_sqlx_migrations` (sqlx, default; renameable via `sqlx.toml` or `dangerous_set_table_name`), `seaql_migrations` (SeaORM, overridable), `__diesel_schema_migrations` (Diesel, fixed), `refinery_schema_history` (refinery). They will not fight over bookkeeping.
2. **Locking doesn't coordinate across tools** — sqlx uses Postgres advisory locks internally (verified in `migrator.rs`); Prisma has its own locking. The two will not serialize each other, so run migration jobs serially (a single K8s Job/initContainer, Prisma first, Rust second), never two kinds concurrently.
3. **One owner per table/object** — keep Prisma as the sole migration owner of shared tables; the Rust side does no runtime DDL for them. If a Rust service legitimately owns tables, put them in their own idempotent SQL set (`CREATE TABLE IF NOT EXISTS`, `DO $$ ... duplicate_object` guards — the same guards the TS migrations already use) run by that service's own migrator (sqlx-cli/refinery) in a separate job; Prisma migrations for *other* objects never touch them.
4. The template's schema is a single `note` model in Postgres schema `template` with `Int` autoincrement ids — trivially hand-written as a sqlx `FromRow` struct, no codegen needed.

→ **Recommendation**: **sqlx 0.9 + sqlx-cli**, with the committed `.sqlx` cache and `SQLX_OFFLINE=true` in dev and CI (`cargo sqlx prepare --check` as a CI gate). Keep **Prisma as the only migration owner** for now; if Rust-owned tables appear, use a separate idempotent migration set with its own tracking table run by a dedicated Job. Do not adopt prisma-client-rust; keep SeaORM 2 as the fallback if the team later wants an ORM, and treat Toasty as one to watch.

---

## 3. Validation + OpenAPI generation

**Candidates and health**

- **utoipa 6.0.0** (2026-09-22) — juhaku; ~18M recent downloads; very active (5.5 May 2026 → 6.0 Sep 2026); MSRV 1.88. **OpenAPI 3.1** output, `yaml` feature for YAML export, `ToSchema`/`ToResponse`/`Responses`, security schemes, recursive schema collection from usage, response bodies via `#[utoipa::path(responses(...))]`, axum integration via **utoipa-axum 0.3.0** (router-level `OpenApiRouter`, same-day release), `axum_extras` for path/query parsing, built-in support for chrono/time/uuid/ulid/jiff types. https://crates.io/crates/utoipa
- **aide 0.15.1** stable, 0.16.0-alpha.4 (2026-04-14) — tamasfe; ~1M downloads; slower, still on alpha for the next version; schemars-based. Not the default.
- **poem-openapi 5.1.16** (2025-07-28) — tied to poem; slowing.
- **okapi 0.7.0** (2024-01-14) — **UNMAINTAINED** (no release in 2.5 years).
- **schemars 1.2.3** (2026-10-12) — active; used by aide/okapi; not needed for utoipa.
- Validation: **garde 0.23.0** (2026-05-23) and **validator 0.21.0** (2026-07-27) both active; **axum-valid 0.25.0** (2026-06-30) provides validated `Json`/`Query`/`Path` extractors over either. New in Oct 2026: **vld 0.4.5 + vld-utoipa 0.4.5** — a Zod-inspired validator whose utoipa integration **auto-generates OpenAPI constraints**; young (0.x), so watch rather than bet the template on it. https://crates.io/crates/garde · https://crates.io/crates/axum-valid · https://crates.io/crates/vld-utoipa

**The TypeBox question — honest answer**: there is no exact Rust equivalent of "one schema → runtime validation + OpenAPI, constraints included". The closest stable pattern is **one struct, multiple derives**: `#[derive(Serialize, Deserialize, ToSchema, garde::Validate)]` with `#[garde(...)]` attributes for runtime rules and `#[schema(...)]` hints for the doc; `axum-valid` performs validation on extraction; `utoipa` documents request *and response* schemas (unlike TypeBox-only request typing, this is actually better for responses). The duplication is limited to constraint hints (length/range/regex) in `#[schema]` vs `#[garde]`; `vld-utoipa` removes it but is immature.

**Error handling**: `thiserror 2.0.21` (2026-09-23) for a domain `ApiError` enum with an `IntoResponse` impl (mirrors the Fastify `setErrorHandler` + `{ error }` shape; consider adding validation detail fields to match the current TypeBox error payload); `anyhow 1.0.104` (2026-07-18) only at the edges (startup/wiring), never in responses.

→ **Recommendation**: **utoipa 6 + utoipa-axum 0.3 + garde 0.23 via axum-valid 0.25**; one struct per DTO carrying serde + `ToSchema` + `Validate`; `thiserror`-based `ApiError` → `IntoResponse`; write `openapi.yaml` at startup in non-production exactly like the TS template does today. Re-evaluate vld-utoipa in a few months; do not use okapi/aide/poem-openapi.

---

## 4. Pact / contract testing

**Status (all verified live)**: the old `pact-foundation/pact-rust` repo is gone (404); everything lives in **`pact-foundation/pact-reference`**, which is **actively maintained through October 2026**: `pact_models 1.3.16` (2026-10-10), `pact_matching 2.0.11` (2026-08-10), `pact_verifier 1.4.7` (2026-08-07), `pact_consumer 1.4.10` (2026-08-07), `pact_verifier_cli 1.3.5` (2026-08-07), `pact-plugin-driver 1.2.6` (2026-10-04), FFI library releases still shipping (0.5.11 on 2026-10-15; release assets are FFI `.so`/`.a` builds). Bus factor: small core (Ronald Holshausen et al.) but foundation-backed and releasing continuously. https://github.com/pact-foundation/pact-reference

**Verification**: `pact_verifier` implements Pact V3 and V4 and supports **provider states** — verified in source, where each interaction runs `execute_state_change` through a `ProviderStateExecutor`. The **CLI** (`pact-verifier` binary) supports exactly our use case: `-f/--file` repeated for **checked-in pact files**, `-d/--dir`, `--state-change-url`, `--state-change-as-query`, `--state-change-teardown`, JSON/JUnit reports, `--base-path`, custom headers, request timeout, plus broker flags (`-b/--broker-url`, token/user/password, `--publish-verification-results`) for later. Distribution: `cargo install pact_verifier_cli` (a binary crate on crates.io), or the Pact Docker images; GitHub release assets are the FFI libs, not the CLI. https://docs.rs/pact_verifier · https://github.com/pact-foundation/pact-reference/blob/master/rust/pact_verifier_cli/README.md

**In-tree vs CLI**: in-tree `pact_verifier` as a dev-dependency pins the version in `Cargo.lock`, runs with the same toolchain, and is one `cargo nextest run --profile pact` away in CI — the most agent-friendly (no extra binary install, no version drift). The CLI stays as the escape hatch for one-off verification or other teams' stacks.

**Consumer side**: the pure-Rust `pact_consumer` DSL is maintained but its docs/examples are thin; since our consumers are TypeScript (and keep generating pacts), **Rust does provider verification only**. If a producer ever needs Rust consumer pacts, revisit.

→ **Recommendation**: Rust providers verify **checked-in pact files** using **pact_verifier 1.4.x as a dev-dependency** (provider states via `--state-change-url`/handler equivalents), with `pact_verifier_cli 1.3.5` as a CI fallback; keep consumer pact generation in the consumers' stacks; publish verification results only when a broker appears.

---

## 5. RabbitMQ / AMQP client

**Candidates and health**

- **lapin 4.12.2** (2026-10-13) — amqp-rs org; ~2.1M recent downloads; very active (4.12.0 Sep → 4.12.2 Oct 2026). THE mature choice. **Built-in automatic connection recovery**: `ConnectionProperties::default().enable_auto_recover()` reconnects and replays exchanges, queues, bindings and consumers; `channel.wait_for_recovery(err).await` blocks until recovery completes. Features: tokio by default, **rustls by default** (platform verifier), `native-tls`/`openssl` optional; runtime-agnostic. https://crates.io/crates/lapin
- **amqprs 2.1.5** (2026-03-19) — gftea; alive but ~20× fewer downloads and thinner examples; no equivalent breadth of production usage.
- **amiquip 0.4.2** (2022-01-12) — **DEAD**, do not use.
- Pools/helpers: **deadpool-lapin 0.14.0** (2026-08-26, active) — usually unnecessary (one shared connection + channels per task is the standard shape); **tokio-retry 0.3.2** (2026-06-09, revived) for custom backoff; **backoff 0.4.0** (2021) — stale.

**Porting the exact TS topology**: topic exchange `main` (durable, `alternate-exchange=nonRouted`, `x-message-ttl=86400000`), topic exchange `deadLetter`, queue `task_queue` (durable, `x-dead-letter-exchange=deadLetter`, `x-message-ttl=3600000`) bound to `main` with `task.#` — in lapin these are `ExchangeDeclareOptions`/`QueueDeclareOptions` + a `FieldTable` of arguments; declare on both API and worker start (idempotent). Consumer: `basic_consume` with manual ack; **prefetch** via `basic_qos(0, n, false)` (the TS code has prefetch commented out — pick a value and keep it configurable). **Publisher confirms**: `channel.confirm_select(...)` then `basic_publish(...).await?.await?` resolves to the broker's `Confirmation` (ack/nack) — this is the confirm-and-log behavior the TS publisher lacks.

**Failure semantics**: the TS worker always acks and, on error, manually republishes a JSON envelope to `deadLetter` with `task_queue.failed` routing key. In Rust, keep that pattern (payload parity with the TS workers) or switch to `basic_nack(requeue=false)` and let the DLX route it — the latter loses the embedded error metadata unless complemented with `x-death` headers. **Keep manual republish + ack** for now.

**Graceful shutdown**: SIGTERM → `basic_cancel` consumers → wait for in-flight handlers → `channel.close()`/`connection.close()` (mirrors `cleanupOnExit`). With auto-recovery enabled, ensure shutdown doesn't race recovery (cancel first, close second).

→ **Recommendation**: **lapin 4.12.x** with `enable_auto_recover()`, publisher confirms (`confirm_select` + confirmation await), `basic_qos` prefetch, topology identical to the TS exchanges/queues, always-ack + manual deadLetter republish, SIGTERM-driven cancel/close. No pooling crate needed initially; add deadpool-lapin only if channel volume proves it necessary.

---

## 6. Observability

**Logging**

- **tracing 0.1.44** (2025-12-18) + **tracing-subscriber 0.3.23** (2026-03-13) — tokio-rs; ubiquitous, stable, active. https://crates.io/crates/tracing-subscriber
- JSON output: `tracing_subscriber::fmt().json()` (flattening with `.flatten_event(true)`, span fields, no ANSI in prod).
- **Two independent log levels (pino parity)**: build the subscriber from two layers, each with its own `EnvFilter` — e.g. a request layer filtered by `REQUEST_LOG_LEVEL` (target `tower_http=info`) and an app layer filtered by `CUSTOM_LOG_LEVEL` — per-layer filtering (`Layer::with_filter`) is exactly designed for this and replaces pino's two-level setup.
- **Redaction**: pino `redact` parity is: (a) `tower_http::sensitive_headers::SetSensitiveRequestHeadersLayer` with `authorization` marked sensitive so `TraceLayer` never logs its value, and (b) never serializing whole header maps in custom spans (the TS prod config redacts all `req.headers` — replicate by logging only method/path/params/status). A custom `Visit` implementation can whitelist fields.
- Request logs come from `TraceLayer` (make_span with method/path/request-id from the tower-http request-id layer, status, latency) — the equivalent of Fastify's `req`/`res` serializers.

**OpenTelemetry (2026 status, from the official README status table)**

- **opentelemetry / opentelemetry_sdk / opentelemetry-otlp / opentelemetry-appender-tracing / opentelemetry-semantic-conventions — all 0.33.1** (2026-10-09). Still **0.x — no 1.0 yet** (any "1.0 is coming" claims in older posts are still unfulfilled; verify against https://github.com/open-telemetry/opentelemetry-rust).
- Maturity per the project README (authoritative as of this date): **Logs API/SDK/OTLP exporter/Appender — Stable; Metrics API/SDK/OTLP — Stable; Traces API/SDK/OTLP — Beta**; Metrics-Prometheus exporter — Beta. Plan accordingly: metrics/logs are production-grade; traces are still flagged Beta (they work fine in practice, but expect churn).
- **tracing-opentelemetry 0.34.0** (2026-09-23) is the tracing bridge; its only `opentelemetry` requirement is `^0.33.0` (verified via the crates.io dependency metadata) — pin both.
- OTLP exporter features: `grpc-tonic` (default) or `http-proto`; logs bridge via `opentelemetry-appender-tracing` (tracing → OTLP logs), which matches the "keep pino-style logs, also ship OTLP" wish.
- **Version churn is the agent hazard here**: the 0.x releases make breaking changes regularly; always pin exact versions and check the compat matrix in the repo README before upgrading.

**RED metrics for axum**

- **axum-prometheus 0.10.1** (2026-07-31) — Ptrskay3; active; compatible with axum 0.8 (crate 0.8–0.10); built on metrics.rs; emits `axum_http_requests_total`, `axum_http_requests_duration_seconds`, `axum_http_requests_pending` plus optional response body size; Prometheus exporter built in. The simplest Prometheus-native RED layer. https://crates.io/crates/axum-prometheus
- Alternatives: **axum-otel-metrics 0.14.1** (2026-07-11) if the cluster is OTLP-first; tower-http 0.7's new `metrics` module for generic HTTP metrics; hand-rolled middleware over the OTel meter API.

→ **Recommendation**: tracing + tracing-subscriber (JSON stdout, two EnvFilter-ed layers for REQUEST_LOG_LEVEL/CUSTOM_LOG_LEVEL, sensitive-headers redaction, TraceLayer for per-request logs) + **axum-prometheus 0.10.1 for RED metrics**. Add the OTLP stack (opentelemetry 0.33.x + tracing-opentelemetry 0.34 + opentelemetry-appender-tracing 0.33) behind a feature flag once a collector exists; pin exact versions because of 0.x churn.

---

## 7. Testing

**Tooling and health**

- **cargo-nextest 0.9.148** (2026-10-08) — nextest-rs; very active; runner worth standardizing on (per-test timeouts, retries, test groups, JUnit XML, `--partition`, profiles). Known limitation: **doctests are not run** — add a separate `cargo test --doc` in CI. https://crates.io/crates/cargo-nextest
- **mockall 0.15.0** (2026-06-28) — asomers; revived after a long gap (0.13.1 → 0.14 → 0.15); still the standard mocking crate, but for this codebase **hand-written fakes behind traits are the better default** (see below).
- **wiremock 0.6.5** (2025-08-24) — LukeMathWalker; no commits since Aug 2025 (slow but stable); **httpmock 0.8.3** (2026-02-04) — more active alternative; **mockito 1.7.2** (2026-02-02) — also viable. **axum-test 21.1.0** (2026-08-20) — drives your axum app in-process (no socket), ideal for feature tests.
- **insta 1.49.0** (2026-10-03) — mitsuhiko; very active; snapshot testing + `cargo-insta`; **rstest 0.27.0** (2026-09-06) — fixtures + parametrized tests.
- **testcontainers 0.28.0** (2026-08-06) + **testcontainers-modules 0.15.0** (2026-02-21) — community modules verified to include **postgres, rabbitmq, redis, valkey** (plus mongo/kafka/localstack/etc.); Docker daemon required (works on GitHub-hosted ubuntu runners). https://github.com/testcontainers/testcontainers-rs-modules-community
- **cargo-llvm-cov 0.9.1** (2026-09-06) — taiki-e; very active; `cargo llvm-cov nextest` is supported natively (verified in the README), LCOV/cobertura reports; branch coverage is nightly-only.

**Unit vs integration vs acceptance (mapping from `test/unit|feature|acceptance`)**

1. **Unit** — `#[cfg(test)] mod tests` next to the code; pure logic; repositories/clients behind traits with hand-written fakes. In Rust the idiom for DI is a trait + generic parameter (`fn new(repo: impl NotesRepository)`) — no module mocking needed; `async fn` in traits is stable (use `async-trait 0.1.92` only for `dyn`/object-safe cases, e.g. mockall dyn mocks).
2. **Feature/integration** — `tests/*.rs`: axum router via **axum-test** with fakes + **wiremock/httpmock** for outbound HTTP; no Docker required, fast.
3. **Acceptance** — `tests/acceptance/*.rs`: full app with **testcontainers** modules (Postgres, RabbitMQ, Redis/Valkey), real migrations, mirroring the TS acceptance suite; gate and parallelize with nextest profiles instead of `#[ignore]`.
4. Coverage: `cargo llvm-cov nextest --workspace` in CI; keep the TS `test:unit:ci` lcov equivalent.

→ **Recommendation**: **nextest as the one test runner** (+ doctests job), hand-written trait fakes by default and mockall only where a fake is noisier, **axum-test + wiremock (or httpmock)** for HTTP, **insta + rstest** for snapshots/fixtures, **testcontainers-modules (postgres/rabbitmq/redis)** for acceptance, and **cargo-llvm-cov + nextest** for coverage.

---

## 8. Docker & build

**Images and health**

- Builder: **`rust:1.99-slim-trixie`** (tags verified on Docker Hub: `1.99-slim-trixie`, `1.99-slim-bookworm`, both exist; stable Rust is 1.99.1 as of today). Debian 13 trixie matches current distroless Debian bases.
- **cargo-chef 0.1.78** (2026-08-12) — LukeMathWalker; still actively released. 2026 guidance (Docker docs, Depot, real ADRs) uses **cargo-chef + BuildKit cache mounts together**: `chef prepare` recipe → `chef cook` in a cached layer keyed on `Cargo.lock`, then the real build with `--mount=type=cache,target=/usr/local/cargo/registry`, `.../cargo/git` and `.../app/target` (copy the binary to an ordinary layer inside the same `RUN` — cache mounts are not layer content). Note the practical conflict: cache mounts **do not persist across GitHub-hosted runners**; layer caching via `--cache-to type=gha,mode=max` does, so keep cargo-chef as the primary CI optimization and treat mounts as a local/self-hosted win. https://docs.docker.com/build/cache/optimize · https://crates.io/crates/cargo-chef
- **sccache 0.18.0** (2026-09-16) — mozilla/sccache; active; optional; has a native **GitHub Actions cache backend** (`gha` feature; `SCCACHE_GHA_ENABLED=true`, sccache-action) for cross-run artifact caching. https://github.com/mozilla/sccache/blob/main/docs/GHA.md
- Runtime base: **`gcr.io/distroless/cc-debian13:nonroot`** (Debian 13 line; current distroless tags) — glibc + cc runtime, includes ca-certificates docs/support, non-root by default. `debian:trixie-slim` if you want a shell for debugging; Alpine/musl works but adds musl caveats (allocator behavior, DNS quirks, build-script toolchain gaps) for little gain — not the boring 2026 default. https://github.com/GoogleContainerTools/distroless
- **TLS**: prefer **rustls everywhere** — sqlx `tls-rustls-ring-webpki`/`...ring-native-roots` features, lapin's default rustls, reqwest `rustls-tls` — so no OpenSSL/libssl is needed in the runtime image at all.
- Release profile: `[profile.release] lto = "thin"`, `codegen-units = 1`, `strip = "symbols"`, `panic = "abort"` (release only — breaks `catch_unwind` and some test tooling); dev loop: `[profile.dev]` default plus `[profile.dev.package."*"] opt-level = 2` to trade one slower first build for much faster edit-check loops across dependencies. https://doc.rust-lang.org/cargo/reference/profiles.html
- Dev loop: **bacon 3.26.0** (2026-09-26, very active; nextest integration, TUI, message filtering, clickable errors) — **cargo-watch 8.5.3** has had no release since Oct 2024 (stale).
- Cross-compilation: **cargo-zigbuild 0.23.4** (2026-09-02, active) for musl/foreign targets if needed; **cross 0.2.5** (2023-02) is stale — prefer **native arm64 GitHub runners** (`ubuntu-24.04-arm`) for multi-arch images.

**Concrete recommended Dockerfile approach**

1. `# syntax=docker/dockerfile:1`; stage `chef` = `rust:1.99-slim-trixie` + `cargo-chef` pinned (installed with `--locked`).
2. `planner` copies the workspace manifests and runs `cargo chef prepare` (recipe is byte-stable across source-only changes).
3. `builder` copies `recipe.json`, runs `cargo chef cook --release --workspace --locked` under cache mounts, then copies sources and runs `cargo build --release --locked`; binary + needed data copied to `/out` in the same `RUN`.
4. Runtime `FROM gcr.io/distroless/cc-debian13:nonroot`, `COPY --from=builder /out/ /app`, `ENTRYPOINT` per image (api/worker images or one image with two CMDs).
5. CI: `docker/build-push-action` with `cache-from/to: type=gha,mode=max`; arm64 via native runners. Enable sccache only if build times prove it necessary.

→ **Recommendation**: one multi-stage Dockerfile using **cargo-chef 0.1.78 + BuildKit cache mounts** on `rust:1.99-slim-trixie`, **distroless cc-debian13:nonroot** runtime, rustls-only TLS, thin LTO + codegen-units=1 + strip; **bacon** for the dev loop; GHA layer-cache for CI; cargo-zigbuild or native arm runners only when multi-arch is actually needed.

---

## 9. Repo tooling & CI

**Task surface & hooks**

- **just 1.59.0** (released **2026-10-15** — very active) — the one obvious command surface (`just check`, `just test`, `just test-acceptance`, `just migrate`, `just fmt`, `just lint`, `just coverage`, `just licenses`, `just sbom`, `just dev`), directly mirroring the current `package.json` scripts. https://crates.io/crates/just
- **cargo xtask**: the pattern (`.cargo/config.toml` alias `xtask = "run --package xtask --"`) is used by Cargo itself, but matklad's spec README explicitly says it is **not officially recommended** and is a polyfill for missing cargo workflows; use it only for logic that outgrows just recipes. **cargo-run-bin 1.7.5** (2025-07-14) pins tool versions declared in `Cargo.toml`.
- **lefthook 2.2.2** (2026-10-13, very active) — single Go binary, replaces husky cleanly (pre-commit format/clippy, pre-push test/check). **cargo-husky 1.5.0** (2020) is **effectively unmaintained** (last push Jul 2024). Plain `.githooks/` + `core.hooksPath` is the zero-dependency fallback; **jdx/hk 2.6.0** (2026-10-12) is an emerging alternative.
- Lint/format: **workspace lints** (`[workspace.lints.rust]` / `[workspace.lints.clippy]` + `[lints] workspace = true` per crate — stable since Rust 1.74), `rustfmt.toml` (stable options only), CI: `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`.

**Supply chain, licenses, SBOM**

- **cargo-deny 0.20.2** (2026-07-09) — EmbarkStudios; active; `deny.toml` covering licenses, advisories, bans, sources — the direct replacement for the custom `check-licenses.ts`. **cargo-audit 0.22.2** (2026-06-05) — rustsec; complementary (CI advisory scan) but cargo-deny's advisory checks make it optional. https://crates.io/crates/cargo-deny
- SBOM: **cargo-cyclonedx 0.5.9** (2026-03-19) — the CycloneDX org's tool (project `cyclonedx-rust-cargo`); **cargo-sbom 0.10.0** (2025-06-17, sbom-rs — repo active, releases slower). **cargo-auditable 0.7.7** (2026-10-02) embeds dependency metadata in the release binary so image scans (syft/grype) can read it — cheap and agent-friendly.

**GitHub Actions (2026 state)**

- **actions-rust-lang/setup-rust-toolchain@v2** (v2.0.0, 2026-09-07) — installs the toolchain, enables caching **via Swatinem/rust-cache internally** (`cache: true` default), sets `build-warnings: deny` by default, provides problem matchers; supersedes calling `Swatinem/rust-cache` (2.9.2, 2026-08-06 — still fine) separately. dtolnay/rust-toolchain remains alive (commits Oct 2026) but the setup action is more opinionated and less YAML.
- **taiki-e/install-action** (actively releasing, Oct 2026) for `nextest` and `cargo-llvm-cov` binaries; **EmbarkStudios/cargo-deny-action 2.1.1** (2026-07-13; tracks cargo-deny 0.20.2); **houseabsolute/actions-rust-cross 1.0.8** (2026-06-21) if cross builds are needed.
- Recommended job layout: `fmt` → `clippy -D warnings` → `test (nextest, unit+feature)` → `acceptance (testcontainers)` → `deny` → `coverage (llvm-cov nextest, lcov upload)` → `docker (buildx, gha cache, multi-arch)`; add `cargo sqlx prepare --check` (needs a Postgres service container or a checked-in `.sqlx` diff check) and `cargo cyclonedx` artifact upload.

→ **Recommendation**: **justfile** as the single command surface (mirror every `package.json` script 1:1), **lefthook 2.x** for hooks, workspace lints + clippy `-D warnings` in CI, **cargo-deny** for licenses/advisories (delete the custom license script's Rust-side equivalent), **cargo-cyclonedx + cargo-auditable** for SBOMs, and GitHub Actions with **setup-rust-toolchain@v2 + taiki-e/install-action + cargo-deny-action + buildx type=gha cache**. Skip xtask unless logic outgrows just.

---

## 10. Config & parity misc

- **Config**: **config-rs 0.15.27** (2026-09-30) — active; layered files+env with serde structs (`Environment::with_prefix("APP")`), the best fit for K8s (env primary) plus `.env` for dev. **figment 0.10.19** — last release **May 2024** (repo touched in Oct 2026 but no releases; semi-dormant — call it out if you see it recommended). **envy 0.4.2** (2021) — unmaintained. **dotenvy 0.15.7** (2023-03-22; repo lightly maintained, pushed Aug 2026) — frozen but ubiquitous and correct for loading `.env` in dev. Pattern: `Config` struct + `dotenvy::dotenv().ok()` + `config::Config::builder()` layering, validate and fail fast at startup, print a redacted config summary.
- **serde camelCase**: `#[serde(rename_all = "camelCase")]` on all DTOs/entities to match Prisma/TypeScript JSON; add `deny_unknown_fields` on request DTOs for TypeBox-like strictness; utoipa reads serde attributes for schema generation.
- **IDs**: this template's Prisma schema uses `Int @id @default(autoincrement())` (**verified** in `prisma/schema.prisma`) — no cuid generation needed for parity. For services where Prisma uses `cuid()` (CUID **v1**, 25-char): the **`cuid` crate 2.0.4** (2026-04-19, mplanchard/cuid-rust; `cuid1()`/`cuid2()` API after the 2.0 reorg) or the standalone `cuid1`/`cuid2` crates; CUID v1 is explicitly kept supported indefinitely by the maintainer. https://crates.io/crates/cuid
- **Postgres `text[]` and Json/Jsonb**: sqlx — `Vec<T>` for arrays, `Json<T>`/`serde_json::Value` (`json` feature) for json/jsonb (verified features); SeaORM 2 handles both (dense entities map PG arrays; 2.0 included array codegen fixes). **chrono vs time**: both feature-gated in sqlx 0.9 — and 0.9's `sqlx.toml` can force the winner when both are enabled (`[macros.preferred-crates] date-time = "chrono"|"time"`). Recommend **chrono 0.4.45** (2026-06-04) for ecosystem parity (most examples/docs); note Prisma `DateTime` → `timestamp(3)` → `NaiveDateTime`. **jiff 0.2.38** (2026-10-06) is the rising star (very active) but **sqlx has no jiff integration as of 0.9** (no jiff entry in the changelog) — utoipa does offer `jiff_0_2`. Watch it.
- **JWT**: **jsonwebtoken 11.1.0** (2026-09-16) — Keats; clearly maintained (9.3 → 10.0 Sep 2025 → 11.0 Jul 2026 → 11.1 Sep 2026). **Heads-up: the 10.x/11.x line is a different API from every 9.x tutorial** — crypto backends are now pluggable (`aws_lc_rs` or `rust_crypto` providers must be installed), 11.0 reworked JWK handling (JWKs creatable from keys, `non_exhaustive` enums, removals). It supports JWKS/JWK decoding, HS/RS/ES/PS/EdDSA — the correct port target for the jose RS256 middleware. **josekit 0.10.3** (2025-05-20, quiet) only if JWE is genuinely needed. https://crates.io/crates/jsonwebtoken
- **Redis/Valkey**: **`redis` (redis-rs) 1.7.1** (2026-09-25) — very active 1.x line (1.5→1.7 in Aug–Sep 2026); tokio async, multiplexed connections, pub/sub, rustls features, works with Valkey. **fred 10.1.0** — last release Feb 2025 and **no commits since Feb 2025 (dormant)**; don't pick it for a new template. **deadpool-redis 0.23.1** (2026-08-26) if you want a pool (usually a single multiplexed connection suffices for cache ops). Keyspace notifications: enable `notify-keyspace-events` server-side and subscribe on `__keyevent@<db>__:<event>` via pub/sub. https://crates.io/crates/redis

→ **Recommendation**: **config-rs + dotenvy + serde structs** (skip figment/envy), camelCase serde everywhere, **cuid crate only if/when cuid models exist** (this repo doesn't need it), **sqlx with chrono + `Vec<T>`/`Json<T>`**, **jsonwebtoken 11 for JWT/JWKS**, and **redis-rs 1.7 (not fred)** for Valkey with deadpool-redis only if pooling becomes necessary.

---

## 11. Agent-friendliness sweep (2025–2026)

- **Toolchain**: current stable is **Rust 1.99.1 (2026-10-15)**; 1.99.0 came 2026-10-01, 1.98.x in Aug/Sep 2026 — the usual 6-week cadence. **Edition 2024** is current (stable since 1.85, Feb 2025). An **edition 2027 is under discussion for early 2027 but explicitly not committed** ("If we get to late 2026 and we have not developed enough interesting stuff … there should not be a 2027 edition… perhaps 2028 or later"), and other participants expect a 2028-era landing — **sources conflict; do not plan around it**. Use `edition = "2024"` now. https://blog.rust-lang.org/2026/10/01/Rust-1.99.0.html
- **Language features that change agent-written code**: `async fn` in traits (1.75) — works for static dispatch/generics; `dyn` still needs `async-trait 0.1.92` or manual boxed futures; **async closures** (1.85); **let-chains** (`if let Some(x) = a && x > 1`, stabilized in **1.88, edition-2024 only** — verified via the 1.88 release notes); `LazyLock` (1.80). These remove a lot of the old boilerplate that agents historically fumbled.
- **AGENTS.md is the standard**: stewarded by the **Agentic AI Foundation (Linux Foundation)**, 60k+ repositories; the dominant context format across tools (Codex, Cursor, Copilot, opencode, Gemini; Claude Code reads CLAUDE.md, commonly a symlink to AGENTS.md). Empirical 2026 studies: repositories with a root AGENTS.md show **~28% lower median wall-clock** and **~16.6% fewer output tokens** on agent tasks (arXiv 2601.20404), and Rust repos lead language-level adoption in one 40k-repo sample. Practical shape from agents.md + studies: commands first (build/test/lint), then a map of the repo, conventions, "what not to do", nested AGENTS.md per subproject when large; median ~900 words. This repo already has AGENTS.md + `docs/agents/` — extend it with: the `just` command table, crate map, sqlx offline rules (`.sqlx` committed, `SQLX_OFFLINE=true`), "run `just check` before finishing", and a pitfalls section (below).
- **Agent tooling for Rust**: several maintained rust-analyzer MCP servers exist in 2026 — **rust-analyzer-mcp 0.4.0** (2026-08-22; hover/defs/refs/diagnostics over MCP), r3bl-rust-analyzer-mcp-server (1.1.x), rustbelt, fcp-rust; **rust-mcp-sdk 2.0.0** (2026-08-27) if you ever build your own. Adoption is still niche; rust-analyzer's built-in CLI (`rust-analyzer diagnostics .`) plus `cargo check --message-format=json` cover most machine-readable feedback needs without extra moving parts. https://crates.io/crates/rust-analyzer-mcp
- **Workspace layout that helps agents**: one library + two thin binaries (`src/bin/api.rs`, `src/bin/worker.rs`) sharing the lib — idiomatic and gives `cargo run --bin api` / `--bin worker` instead of a `DEPLOYMENT_MODE` switch (keep the env var only if the deployment story requires one image/entrypoint); small, focused crates (parallel compile, crisp boundaries); `rust-toolchain.toml` pinned (1.99.1); workspace lints; `.cargo/config.toml` aliases (`c`, `t`, `l`); a justfile that mirrors the old npm scripts one-for-one; examples/ for each infra client (RabbitMQ, Redis, DB) as copy-paste references; domain docs (GLOSSARY.md, ADRs) as this repo already prescribes.
- **Pitfalls agents hit in Rust (encode guardrails)**: async trait dyn dispatch (prefer generics; `async-trait` only when needed); `Arc<Mutex<…>>` overuse where a channel/task is better; missing `Send + 'static` on spawned tasks; axum extractor ordering (body-consuming extractor must be last); lifetimes in handlers (own the data; don't borrow request fields into returned futures); feature unification surprises in workspaces (use the 2024-edition resolver; beware default features enabling TLS/DB backends you didn't intend); sqlx macros failing without a DB (commit `.sqlx`, set `SQLX_OFFLINE=true`, and keep `cargo sqlx prepare --check` green); orphan rules (newtypes for external trait impls); tower layer ordering (CORS/early-reject layers outermost); `clippy --fix` loops (run `just lint` after).
- **Compile-loop accelerators**: `cargo check` over `cargo build`; nextest for parallel test feedback; bacon for continuous check; sccache for CI/cross-machine; small crates + `[profile.dev.package."*"] opt-level` to stop one edit rebuilding the world; `cargo check --message-format=json` for machine-readable errors.

→ **Recommendation**: pin Rust 1.99.1 / edition 2024 in `rust-toolchain.toml`, treat AGENTS.md as *the* guidance artifact (commands first, crate map, sqlx-offline rules, pitfalls list, ~900 words), restructure to one lib + `api`/`worker` bins, make `just` the only command surface agents need, and encode guardrails as workspace lints + cargo-deny + nextest profiles + a committed `.sqlx` cache so the compile loop never needs a live database.

---

### Cross-cutting defaults (one-screen summary)

| Concern | Pick | Version (date) |
|---|---|---|
| HTTP framework | axum + tower-http | 0.8.9 (Apr 2026) / 0.7.1 (Aug 2026) |
| Async runtime | tokio | 1.53.3 (Oct 2026) |
| DB | sqlx (+ .sqlx offline cache) | 0.9.0 (May 2026) |
| Migrations | Prisma-owned (unchanged); sqlx only if Rust owns tables | — |
| Validation | garde via axum-valid | 0.23.0 (May 2026) / 0.25.0 (Jun 2026) |
| OpenAPI | utoipa + utoipa-axum | 6.0.0 / 0.3.0 (Sep 2026) |
| Errors | thiserror + IntoResponse | 2.0.21 (Sep 2026) |
| AMQP | lapin (auto-recover + confirms) | 4.12.2 (Oct 2026) |
| Logging | tracing + tracing-subscriber (JSON, 2 filters) | 0.1.44 / 0.3.23 |
| Metrics | axum-prometheus | 0.10.1 (Jul 2026) |
| OTel | opentelemetry + tracing-opentelemetry (feature-gated) | 0.33.1 / 0.34.0 (Oct/Sep 2026) |
| Pact | pact_verifier in-tree (+ CLI fallback) | 1.4.7 / 1.3.5 (Aug 2026) |
| Testing | nextest, axum-test, insta, testcontainers-modules, llvm-cov | see §7 |
| Docker | cargo-chef + cache mounts → distroless cc-debian13:nonroot | cargo-chef 0.1.78 (Aug 2026) |
| Dev loop | bacon (+ sccache in CI) | 3.26.0 (Sep 2026) |
| Tasks | just (+ lefthook hooks) | 1.59.0 (Oct 2026) / 2.2.2 (Oct 2026) |
| Supply chain | cargo-deny + cargo-cyclonedx + cargo-auditable | 0.20.2 / 0.5.9 / 0.7.7 |
| CI | setup-rust-toolchain@v2 + taiki-e/install-action + cargo-deny-action | v2.0.0 (Sep 2026) |
| Config | config-rs + dotenvy | 0.15.27 (Sep 2026) / 0.15.7 |
| JWT | jsonwebtoken | 11.1.0 (Sep 2026) |
| GraphQL (opt-in) | async-graphql + async-graphql-axum | 7.2.1 (stable; 8.0.0-rc.5 is a release candidate) |
| Redis | redis (redis-rs) | 1.7.1 (Sep 2026) |
| Toolchain | Rust 1.99.1, edition 2024 | 2026-10-15 |

Dead / avoid: prisma-client-rust (archived), amiquip (2022), okapi (2024), cargo-husky (2020), envy (2021), backoff (2021), cross (2023, stale), fred (dormant since Feb 2025), cargo-watch (2024, stale), Rocket (0.5.1, 2024), figment (no release since May 2024 — semi-dormant), async-std (deprecated — tokio everywhere).
