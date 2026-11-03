//! Postgres access: the sqlx pool the app shares with handlers and repositories,
//! plus the database readiness probe.
//!
//! Pool creation is lazy ([`Database::connect_lazy`]): startup validates
//! `DATABASE_URL` but never connects, so the service boots while Postgres is
//! down and `/readyz` reports the outage through [`Database::check`].
//!
//! Queries go through the `sqlx::query*` macros and are checked at compile time
//! against the committed `.sqlx` cache (ADR-0003): `cargo build`/`cargo test`
//! never need a database. `just sqlx-prepare` refreshes the cache and `just
//! sqlx-check` (part of `just check`) fails when it is stale.

use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use crate::health::{ReadinessCheck, ReadinessFuture};

/// How long one readiness probe may take before the database is reported
/// unhealthy. Requests never wait longer than this on `/readyz`.
pub const READINESS_TIMEOUT: Duration = Duration::from_secs(2);

/// The Postgres connection pool (cloning shares the same pool).
#[derive(Clone, Debug)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    /// Creates the pool without opening a connection; the first query (or
    /// readiness probe) connects. Rejects a malformed `DATABASE_URL`.
    pub fn connect_lazy(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new().connect_lazy(database_url)?;
        Ok(Self { pool })
    }

    /// The pool handlers and repositories run queries through.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Closes the pool and waits for checked-out connections to return
    /// (registered as a shutdown cleanup hook by `main`).
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

impl ReadinessCheck for Database {
    fn name(&self) -> &str {
        "database"
    }

    fn check(&self) -> ReadinessFuture<'_> {
        Box::pin(async move {
            // Compile-time checked against the offline cache, so the probe runs
            // through the same macro path as repository queries.
            let probe = sqlx::query_scalar!("SELECT 1").fetch_one(&self.pool);
            match tokio::time::timeout(READINESS_TIMEOUT, probe).await {
                Ok(Ok(Some(1))) => Ok(()),
                Ok(Ok(value)) => Err(format!("unexpected readiness probe result: {value:?}")),
                Ok(Err(error)) => Err(error.to_string()),
                Err(_) => Err(format!(
                    "database did not answer within {}ms",
                    READINESS_TIMEOUT.as_millis()
                )),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// Nothing listens on port 1; TCP connect fails immediately.
    const UNREACHABLE_DATABASE_URL: &str = "postgres://user:pass@127.0.0.1:1/db";

    #[test]
    fn malformed_database_url_fails_fast() {
        let error = Database::connect_lazy("not a database url").unwrap_err();

        assert!(!error.to_string().is_empty());
    }

    #[tokio::test]
    async fn readiness_check_fails_when_the_database_is_unreachable() {
        let database = Database::connect_lazy(UNREACHABLE_DATABASE_URL).unwrap();

        let started = Instant::now();
        let result =
            tokio::time::timeout(READINESS_TIMEOUT + Duration::from_secs(5), database.check())
                .await
                .expect("readiness probe must be bounded by a timeout");

        assert!(
            result.is_err(),
            "unreachable database must fail: {result:?}"
        );
        assert!(
            started.elapsed() < READINESS_TIMEOUT + Duration::from_secs(5),
            "probe must not hang, took {:?}",
            started.elapsed()
        );
    }
}
