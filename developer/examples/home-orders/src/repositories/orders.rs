//! Orders repository. [`OrdersRepository`] is the seam the REST service and the
//! GraphQL data loader depend on; tests swap in hand-written fakes (the house
//! idiom — no module mocking in Rust).

use std::future::Future;
use std::pin::Pin;

use sqlx::PgPool;
use thiserror::Error;

use crate::models::order::Order;

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

/// What the orders service and the GraphQL loader need from storage.
pub trait OrdersRepository: Send + Sync + 'static {
    /// One page of orders, newest first.
    fn find_page(&self, page: u32, per_page: u32) -> RepositoryFuture<'_, Vec<Order>>;

    /// One page of one user's orders, newest first. The default filters a
    /// global page (for fakes); the Postgres implementation pushes the owner
    /// into the query so pagination stays correct.
    fn find_page_by_user(
        &self,
        user_id: i64,
        page: u32,
        per_page: u32,
    ) -> RepositoryFuture<'_, Vec<Order>> {
        let orders = self.find_page(page, per_page);
        Box::pin(async move {
            Ok(orders
                .await?
                .into_iter()
                .filter(|order| order.user_id == user_id)
                .collect())
        })
    }

    /// One order by id.
    fn find_by_id(&self, id: i64) -> RepositoryFuture<'_, Option<Order>>;

    /// Every order whose id is in `ids`; the GraphQL loader batches into this.
    fn find_by_ids(&self, ids: &[i64]) -> RepositoryFuture<'_, Vec<Order>>;
}

/// Postgres-backed orders repository (sqlx, compile-time checked against the
/// committed `.sqlx` cache).
#[derive(Debug, Clone)]
pub struct PgOrdersRepository {
    pool: PgPool,
}

impl PgOrdersRepository {
    /// Repository over the shared pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Inserts an order. Used by `db_seed` and the Pact provider states; the
    /// service exposes no create endpoint.
    pub async fn insert(
        &self,
        user_id: i64,
        total: i64,
        status: &str,
    ) -> Result<Order, sqlx::Error> {
        sqlx::query_as!(
            Order,
            r#"INSERT INTO orders ("userId", total, status, "createdAt")
               VALUES ($1, $2, $3, CURRENT_TIMESTAMP)
               RETURNING id, "userId" AS user_id, total, status, "createdAt" AS created_at"#,
            user_id,
            total,
            status
        )
        .fetch_one(&self.pool)
        .await
    }
}

impl OrdersRepository for PgOrdersRepository {
    fn find_page(&self, page: u32, per_page: u32) -> RepositoryFuture<'_, Vec<Order>> {
        Box::pin(async move {
            let offset = i64::from(page.saturating_sub(1)) * i64::from(per_page);
            let limit = i64::from(per_page);

            let orders = sqlx::query_as!(
                Order,
                r#"SELECT id, "userId" AS user_id, total, status, "createdAt" AS created_at
                   FROM orders
                   ORDER BY id
                   LIMIT $1 OFFSET $2"#,
                limit,
                offset
            )
            .fetch_all(&self.pool)
            .await?;

            Ok(orders)
        })
    }

    fn find_page_by_user(
        &self,
        user_id: i64,
        page: u32,
        per_page: u32,
    ) -> RepositoryFuture<'_, Vec<Order>> {
        Box::pin(async move {
            let offset = i64::from(page.saturating_sub(1)) * i64::from(per_page);
            let limit = i64::from(per_page);

            let orders = sqlx::query_as!(
                Order,
                r#"SELECT id, "userId" AS user_id, total, status, "createdAt" AS created_at
                   FROM orders
                   WHERE "userId" = $1
                   ORDER BY id
                   LIMIT $2 OFFSET $3"#,
                user_id,
                limit,
                offset
            )
            .fetch_all(&self.pool)
            .await?;

            Ok(orders)
        })
    }

    fn find_by_id(&self, id: i64) -> RepositoryFuture<'_, Option<Order>> {
        Box::pin(async move {
            let order = sqlx::query_as!(
                Order,
                r#"SELECT id, "userId" AS user_id, total, status, "createdAt" AS created_at
                   FROM orders
                   WHERE id = $1"#,
                id
            )
            .fetch_optional(&self.pool)
            .await?;

            Ok(order)
        })
    }

    fn find_by_ids(&self, ids: &[i64]) -> RepositoryFuture<'_, Vec<Order>> {
        let ids = ids.to_vec();
        Box::pin(async move {
            let orders = sqlx::query_as!(
                Order,
                r#"SELECT id, "userId" AS user_id, total, status, "createdAt" AS created_at
                   FROM orders
                   WHERE id = ANY($1)
                   ORDER BY id"#,
                &ids
            )
            .fetch_all(&self.pool)
            .await?;

            Ok(orders)
        })
    }
}
