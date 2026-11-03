//! `just db-seed`: destructive, dev-only seeding of the `users` table.
//! Refuses to run with `APP_ENV=production`; truncates `users` (restarting the
//! id sequence) and inserts the reference users so the REST and GraphQL
//! endpoints have data to serve. Uses the same config/pool/repository stack as
//! the service app.

use std::process::ExitCode;

use home_users::config::AppConfig;
use home_users::database::Database;
use home_users::repositories::users::PgUsersRepository;

/// Reference users inserted by the seed (name, email).
const SEED_USERS: [(&str, &str); 5] = [
    ("Ada Lovelace", "ada@example.test"),
    ("Grace Hopper", "grace@example.test"),
    ("Alan Turing", "alan@example.test"),
    ("Margaret Hamilton", "margaret@example.test"),
    ("Linus Torvalds", "linus@example.test"),
];

#[tokio::main]
async fn main() -> ExitCode {
    let config = match AppConfig::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("failed to load configuration: {error}");
            return ExitCode::FAILURE;
        }
    };

    if config.app_env.is_production() {
        eprintln!("db-seed is destructive and refuses to run with APP_ENV=production");
        return ExitCode::FAILURE;
    }

    let database = match Database::connect_lazy(&config.database_url) {
        Ok(database) => database,
        Err(error) => {
            eprintln!("invalid DATABASE_URL: {error}");
            return ExitCode::FAILURE;
        }
    };

    match seed(&database).await {
        Ok(inserted) => {
            println!("seeded {inserted} users into the `users` table");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("seeding failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Truncates `users` and inserts [`SEED_USERS`]; returns the number inserted.
async fn seed(database: &Database) -> Result<usize, sqlx::Error> {
    sqlx::query!("TRUNCATE TABLE users RESTART IDENTITY")
        .execute(database.pool())
        .await?;

    let repository = PgUsersRepository::new(database.pool().clone());
    for (name, email) in SEED_USERS {
        repository.insert(name, email).await?;
    }

    Ok(SEED_USERS.len())
}
