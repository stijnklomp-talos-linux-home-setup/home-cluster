//! `just db-seed`: destructive, dev-only seeding of the `orders` table.
//! Refuses to run with `APP_ENV=production`; truncates `orders` (restarting the
//! id sequence) and inserts reference orders so the REST and GraphQL endpoints
//! have data to serve. `userId` values are references to the users subgraph's
//! seeded users (ids 1–5 in `home-users`). Uses the same config/pool/repository
//! stack as the service app.

use std::process::ExitCode;

use home_orders::config::AppConfig;
use home_orders::database::Database;
use home_orders::repositories::orders::PgOrdersRepository;

/// Reference orders inserted by the seed (userId, total in cents, status).
/// Three users, five orders: the cross-service query has several users to
/// resolve and each user appears at least once.
const SEED_ORDERS: [(i64, i64, &str); 5] = [
    (1, 12_500, "paid"),
    (1, 3_400, "shipped"),
    (2, 89_900, "pending"),
    (3, 250, "paid"),
    (3, 7_600, "cancelled"),
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
            println!("seeded {inserted} orders into the `orders` table");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("seeding failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Truncates `orders` and inserts [`SEED_ORDERS`]; returns the number inserted.
async fn seed(database: &Database) -> Result<usize, sqlx::Error> {
    sqlx::query!("TRUNCATE TABLE orders RESTART IDENTITY")
        .execute(database.pool())
        .await?;

    let repository = PgOrdersRepository::new(database.pool().clone());
    for (user_id, total, status) in SEED_ORDERS {
        repository.insert(user_id, total, status).await?;
    }

    Ok(SEED_ORDERS.len())
}
