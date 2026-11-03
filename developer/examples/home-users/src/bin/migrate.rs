//! `migrate`: apply the embedded sqlx migrations and exit.
//!
//! The Helm chart runs this as a post-install/post-upgrade Job, so a
//! deployment never serves traffic against an unmigrated schema. It uses the
//! same `DATABASE_URL` Secret as the service and sqlx's `_sqlx_migrations`
//! table, so re-runs are idempotent.

use std::process::ExitCode;

use home_users::config::AppConfig;
use home_users::database::Database;

#[tokio::main]
async fn main() -> ExitCode {
    let config = match AppConfig::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("failed to load configuration: {error}");
            return ExitCode::FAILURE;
        }
    };

    let database = match Database::connect_lazy(&config.database_url) {
        Ok(database) => database,
        Err(error) => {
            eprintln!("invalid DATABASE_URL: {error}");
            return ExitCode::FAILURE;
        }
    };

    match sqlx::migrate!("./migrations").run(database.pool()).await {
        Ok(()) => {
            println!("migrations applied");
            database.close().await;
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("migration failed: {error}");
            ExitCode::FAILURE
        }
    }
}
