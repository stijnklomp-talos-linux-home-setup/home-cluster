//! Users controllers. Input extraction lives here (`ValidatedQuery`, see
//! [`crate::validation`]); invalid input never reaches the service and comes
//! back as problem+json 400 (ADR-0005).

use axum::Json;
use axum::extract::{Path, State};

use crate::models::user::{GetUsersQuery, User, UsersResponse};
use crate::problem::{ApiError, Problem};
use crate::server::AppState;
use crate::validation::ValidatedQuery;

/// `GET /v1/users`: one page of users.
#[utoipa::path(
    get,
    path = "/v1/users",
    tag = "users",
    params(GetUsersQuery),
    responses(
        (status = 200, description = "A page of users", body = UsersResponse),
        (status = 400, description = "Invalid query parameters", body = Problem),
        (status = 500, description = "Internal server error", body = Problem),
    )
)]
pub async fn get_users(
    State(state): State<AppState>,
    ValidatedQuery(query): ValidatedQuery<GetUsersQuery>,
) -> Result<Json<UsersResponse>, ApiError> {
    let users = state.users()?.get_users(&query).await?;

    Ok(Json(UsersResponse { users }))
}

/// `GET /v1/users/{id}`: one user, or problem+json 404.
#[utoipa::path(
    get,
    path = "/v1/users/{id}",
    tag = "users",
    params(("id" = i64, Path, description = "User id.")),
    responses(
        (status = 200, description = "The user", body = User),
        (status = 404, description = "No user with that id", body = Problem),
        (status = 500, description = "Internal server error", body = Problem),
    )
)]
pub async fn get_user(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<User>, ApiError> {
    state
        .users()?
        .get_user(id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}
