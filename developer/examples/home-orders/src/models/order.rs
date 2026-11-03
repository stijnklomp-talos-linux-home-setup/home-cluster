//! DTOs for the `orders` subgraph (ticket #16).
//!
//! The same `Order` row backs both API shapes: the REST response ([`Order`])
//! and the federated GraphQL entity in [`crate::graphql::schema`]. Field names
//! stay snake_case in Rust; `serde` and `utoipa` expose them as camelCase, and
//! the SQL aliases the camelCase columns.
//!
//! `user_id` is a *reference* to the `users` subgraph's `User` entity, not a
//! foreign key: orders owns its own database and resolves the user through
//! federation (`Order.user` returns a `User` representation).

use chrono::NaiveDateTime;
use garde::Validate;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::{IntoParams, ToSchema};

/// The statuses an order can be in; the database enforces the same set with a
/// CHECK constraint.
pub const ORDER_STATUSES: [&str; 4] = ["pending", "paid", "shipped", "cancelled"];

/// An order (one `orders` table row).
#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    /// Federation key and primary key.
    pub id: i64,
    /// Reference to the `User` entity in the users subgraph (federation key).
    pub user_id: i64,
    /// Order total in minor units (cents); money is never a float.
    pub total: i64,
    /// One of [`ORDER_STATUSES`].
    pub status: String,
    /// Creation timestamp.
    pub created_at: NaiveDateTime,
}

/// `GET /v1/orders` query parameters.
///
/// List contract shared with `users`: `page` minimum 1, `perPage` maximum
/// 100, both required.
#[derive(Debug, Clone, Deserialize, Validate, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct GetOrdersQuery {
    /// Page number, starting at 1.
    #[garde(range(min = 1))]
    #[param(minimum = 1)]
    pub page: u32,
    /// Orders per page, at most 100 (0 returns an empty page).
    #[garde(range(max = 100))]
    #[param(minimum = 0, maximum = 100)]
    pub per_page: u32,
}

/// `GET /v1/orders` response body.
#[derive(Debug, Serialize, ToSchema)]
pub struct OrdersResponse {
    /// The requested page of orders.
    pub orders: Vec<Order>,
}
