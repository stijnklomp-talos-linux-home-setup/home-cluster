//! Acceptance tests: the real app against real infrastructure.
//!
//! These tests are the full stack: the real router over the real Postgres
//! repository, the real Valkey cache client and the real AMQP
//! publisher/consumer pair, driven in-process (axum-test) against
//! Postgres/Valkey/RabbitMQ from compose. They live in their own cargo test
//! target behind the `acceptance` feature (see `Cargo.toml`), so the default
//! suite never builds or runs them.
//!
//! Run them once through the `test` compose profile (the `justfile` gate):
//!
//! ```text
//! just test-acceptance
//! ```
//!
//! or interactively against an idle `acceptance` stack:
//!
//! ```text
//! just acceptance
//! docker compose --profile acceptance exec acceptance \
//!     cargo nextest run --features acceptance --profile acceptance
//! docker compose --profile acceptance exec acceptance \
//!     cargo nextest run --features acceptance --profile acceptance -E 'test(cache_round_trips)'
//! docker compose --profile acceptance down -v
//! ```
//!
//! The harness reads the infrastructure endpoints from the environment
//! (`DATABASE_URL`, `CACHE_*`, `RABBIT_*`); both compose profiles set them to
//! the in-network service names, so host port remaps never leak into the
//! tests. Migrations run in-harness: every test starts from
//! [`migrated_database`], and `sqlx::migrate!` applies only pending migrations
//! (tracked in `_sqlx_migrations`), which keeps repeated runs deterministic.
//! The `acceptance` nextest profile runs the tests serially (one shared
//! database and broker).

// `tests/common/` is shared with the real-broker integration-test crates;
// their plain `mod common;` cannot reach it from this test root.
#[path = "../common/mod.rs"]
mod common;

mod cache;
mod mq;
mod readiness;
mod users;

use home_users::config::{CacheConfig, RabbitConfig, RabbitTransport};
use home_users::database::Database;

/// Reads a required environment variable, with a message pointing at the
/// intended entry points.
pub(crate) fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!(
            "{name} is not set; run the acceptance suite via `just test-acceptance` \
             or the compose `acceptance` profile"
        )
    })
}

/// The Postgres DSN the suite runs against.
pub(crate) fn database_url() -> String {
    required_env("DATABASE_URL")
}

/// RabbitMQ settings from the environment, always enabled (the acceptance
/// stack has a real broker).
pub(crate) fn rabbit_config() -> RabbitConfig {
    RabbitConfig {
        enabled: true,
        host: required_env("RABBIT_HOST"),
        port: required_env("RABBIT_PORT")
            .parse()
            .expect("RABBIT_PORT must be a port number"),
        user: required_env("RABBIT_USER"),
        password: required_env("RABBIT_PASSWORD"),
        transport: RabbitTransport::Amqp,
    }
}

/// Valkey settings from the environment, always enabled.
pub(crate) fn cache_config() -> CacheConfig {
    CacheConfig {
        disabled: false,
        host: required_env("CACHE_HOST"),
        port: required_env("CACHE_PORT")
            .parse()
            .expect("CACHE_PORT must be a port number"),
        password: required_env("CACHE_PASSWORD"),
    }
}

/// A name no other run uses, so repeated runs and parallel stacks never see
/// each other's rows, keys or queues.
pub(crate) fn unique_label(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is set")
        .as_nanos();
    format!("{prefix}-{}-{nanos}", std::process::id())
}

/// A lazy Postgres pool with every migration applied.
///
/// `sqlx::migrate!` is idempotent — it applies only pending migrations and
/// records them in `_sqlx_migrations` — so running it in every test keeps the
/// suite self-contained on a fresh compose volume.
pub(crate) async fn migrated_database() -> Database {
    let database =
        Database::connect_lazy(&database_url()).expect("DATABASE_URL must be a valid Postgres URL");
    sqlx::migrate!("./migrations")
        .run(database.pool())
        .await
        .expect("the acceptance migrations must apply");
    database
}
