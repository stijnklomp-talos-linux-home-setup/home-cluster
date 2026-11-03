//! Orders controllers. Input extraction lives here (`ValidatedQuery`, see
//! [`crate::validation`]); invalid input never reaches the service and comes
//! back as problem+json 400 (ADR-0005).

use axum::Json;
use axum::extract::{Path, State};

use crate::models::order::{GetOrdersQuery, Order, OrdersResponse};
use crate::problem::{ApiError, Problem};
use crate::server::AppState;
use crate::validation::ValidatedQuery;

/// `GET /v1/orders`: one page of orders.
#[utoipa::path(
    get,
    path = "/v1/orders",
    tag = "orders",
    params(GetOrdersQuery),
    responses(
        (status = 200, description = "A page of orders", body = OrdersResponse),
        (status = 400, description = "Invalid query parameters", body = Problem),
        (status = 500, description = "Internal server error", body = Problem),
    )
)]
pub async fn get_orders(
    State(state): State<AppState>,
    ValidatedQuery(query): ValidatedQuery<GetOrdersQuery>,
) -> Result<Json<OrdersResponse>, ApiError> {
    let orders = state.orders()?.get_orders(&query).await?;

    Ok(Json(OrdersResponse { orders }))
}

/// `GET /v1/orders/{id}`: one order, or problem+json 404.
#[utoipa::path(
    get,
    path = "/v1/orders/{id}",
    tag = "orders",
    params(("id" = i64, Path, description = "Order id.")),
    responses(
        (status = 200, description = "The order", body = Order),
        (status = 404, description = "No order with that id", body = Problem),
        (status = 500, description = "Internal server error", body = Problem),
    )
)]
pub async fn get_order(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Order>, ApiError> {
    state
        .orders()?
        .get_order(id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}
