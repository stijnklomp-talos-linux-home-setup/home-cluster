//! The orders slice against real Postgres: rows written through the repository
//! are read back through the REST endpoints, exactly as `main` wires them.

use std::sync::Arc;

use axum::http::StatusCode;
use axum_test::TestServer;

use home_orders::database::Database;
use home_orders::repositories::orders::PgOrdersRepository;
use home_orders::server::{AppState, build_router};
use home_orders::services::orders::OrdersService;

use crate::migrated_database;

/// The app as `main` assembles it: database handle, readiness check and the
/// Postgres-backed orders service.
fn app(database: &Database) -> TestServer {
    let mut state = AppState {
        database: Some(database.clone()),
        ..AppState::default()
    };
    state.readiness.register(database.clone());
    state.orders = Some(Arc::new(OrdersService::new(Arc::new(
        PgOrdersRepository::new(database.pool().clone()),
    ))));
    TestServer::new(build_router(state))
}

#[tokio::test]
async fn orders_flow_round_trips_through_real_postgres() {
    let database = migrated_database().await;
    let repository = PgOrdersRepository::new(database.pool().clone());

    let inserted = repository
        .insert(1, 42_500, "paid")
        .await
        .expect("insert the acceptance order");

    let server = app(&database);
    let response = server.get("/v1/orders?page=1&perPage=100").await;
    assert_eq!(response.status_code(), StatusCode::OK);
    let body = response.json::<serde_json::Value>();
    let orders = body["orders"].as_array().expect("orders must be an array");
    let found = orders
        .iter()
        .find(|order| order["id"].as_i64() == Some(inserted.id))
        .expect("the inserted order must be on the first page");
    assert_eq!(found["userId"], 1);
    assert_eq!(found["total"], 42_500);
    assert_eq!(found["status"], "paid");

    let response = server.get(&format!("/v1/orders/{}", inserted.id)).await;
    assert_eq!(response.status_code(), StatusCode::OK);
    let fetched = response.json::<serde_json::Value>();
    assert_eq!(fetched["id"], inserted.id);
    assert_eq!(fetched["userId"], 1);
    assert_eq!(fetched["total"], 42_500);
    assert_eq!(fetched["createdAt"], found["createdAt"]);
}

#[tokio::test]
async fn unknown_order_is_problem_json_404_through_the_real_router() {
    let server = app(&migrated_database().await);

    let response = server.get("/v1/orders/9223372036854775807").await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some(home_orders::problem::PROBLEM_JSON_CONTENT_TYPE)
    );
}
