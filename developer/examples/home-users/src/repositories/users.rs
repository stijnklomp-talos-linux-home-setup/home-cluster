//! Users repository. [`UsersRepository`] is the seam the REST service and the
//! GraphQL data loader depend on; tests swap in hand-written fakes (the house
//! idiom — no module mocking in Rust).

use std::future::Future;
use std::pin::Pin;

use sqlx::PgPool;
use thiserror::Error;

use crate::models::user::User;

/// Boxed future returned by repository methods (mirrors `ReadinessFuture`).
pub type RepositoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, RepositoryError>> + Send + 'a>>;

/// Storage failure; the service layer maps it to a problem+json 500.
#[derive(Debug, Error)]
pub enum RepositoryError {
    /// The database refused or failed the operation.
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
}

/// What the users service and the GraphQL loader need from storage.
pub trait UsersRepository: Send + Sync + 'static {
    /// One page of users, ordered by id.
    fn find_page(&self, page: u32, per_page: u32) -> RepositoryFuture<'_, Vec<User>>;

    /// One user by id.
    fn find_by_id(&self, id: i64) -> RepositoryFuture<'_, Option<User>>;

    /// Every user whose id is in `ids`; the GraphQL loader batches into this.
    fn find_by_ids(&self, ids: &[i64]) -> RepositoryFuture<'_, Vec<User>>;
}

/// Postgres-backed users repository (sqlx, compile-time checked against the
/// committed `.sqlx` cache).
#[derive(Debug, Clone)]
pub struct PgUsersRepository {
    pool: PgPool,
}

impl PgUsersRepository {
    /// Repository over the shared pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Inserts a user. Used by `db_seed` and the Pact provider states; the
    /// service exposes no create endpoint.
    pub async fn insert(&self, name: &str, email: &str) -> Result<User, sqlx::Error> {
        sqlx::query_as!(
            User,
            r#"INSERT INTO users (name, email, "createdAt")
               VALUES ($1, $2, CURRENT_TIMESTAMP)
               RETURNING id, email, name, "createdAt" AS created_at"#,
            name,
            email
        )
        .fetch_one(&self.pool)
        .await
    }
}

impl UsersRepository for PgUsersRepository {
    fn find_page(&self, page: u32, per_page: u32) -> RepositoryFuture<'_, Vec<User>> {
        Box::pin(async move {
            let offset = i64::from(page.saturating_sub(1)) * i64::from(per_page);
            let limit = i64::from(per_page);

            let users = sqlx::query_as!(
                User,
                r#"SELECT id, email, name, "createdAt" AS created_at
                   FROM users
                   ORDER BY id
                   LIMIT $1 OFFSET $2"#,
                limit,
                offset
            )
            .fetch_all(&self.pool)
            .await?;

            Ok(users)
        })
    }

    fn find_by_id(&self, id: i64) -> RepositoryFuture<'_, Option<User>> {
        Box::pin(async move {
            let user = sqlx::query_as!(
                User,
                r#"SELECT id, email, name, "createdAt" AS created_at
                   FROM users
                   WHERE id = $1"#,
                id
            )
            .fetch_optional(&self.pool)
            .await?;

            Ok(user)
        })
    }

    fn find_by_ids(&self, ids: &[i64]) -> RepositoryFuture<'_, Vec<User>> {
        let ids = ids.to_vec();
        Box::pin(async move {
            let users = sqlx::query_as!(
                User,
                r#"SELECT id, email, name, "createdAt" AS created_at
                   FROM users
                   WHERE id = ANY($1)
                   ORDER BY id"#,
                &ids
            )
            .fetch_all(&self.pool)
            .await?;

            Ok(users)
        })
    }
}
