//! DTOs for the `users` reference subgraph (ticket #14).
//!
//! The same `User` row backs both API shapes: the REST response
//! ([`User`]) and the federated GraphQL entity in [`crate::graphql::schema`].
//! Field names stay snake_case in Rust; `serde` and `utoipa` expose them as
//! camelCase, and the SQL aliases the camelCase columns.

use chrono::NaiveDateTime;
use garde::Validate;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::{IntoParams, ToSchema};

/// A user (one `users` table row).
#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct User {
    /// Federation key and primary key.
    pub id: i64,
    /// Contact address; unique.
    pub email: String,
    /// Display name.
    pub name: String,
    /// Creation timestamp.
    pub created_at: NaiveDateTime,
}

/// `GET /v1/users` query parameters.
///
/// TypeBox parity with the template's list contract: `page` minimum 1,
/// `perPage` maximum 100, both required.
#[derive(Debug, Clone, Deserialize, Validate, IntoParams, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct GetUsersQuery {
    /// Page number, starting at 1.
    #[garde(range(min = 1))]
    #[param(minimum = 1)]
    pub page: u32,
    /// Users per page, at most 100 (0 returns an empty page).
    #[garde(range(max = 100))]
    #[param(minimum = 0, maximum = 100)]
    pub per_page: u32,
}

/// `GET /v1/users` response body.
#[derive(Debug, Serialize, ToSchema)]
pub struct UsersResponse {
    /// The requested page of users.
    pub users: Vec<User>,
}
