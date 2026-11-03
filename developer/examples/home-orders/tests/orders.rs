//! Feature tests for the orders REST slice: the endpoints through the real
//! router, driven in-process by axum-test, with a hand-written fake repository
//! behind the service (the house idiom — no database).

use std::sync::Arc;

use axum::http::{StatusCode, header};
use axum_test::TestServer;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use home_orders::models::order::Order;
use home_orders::problem::PROBLEM_JSON_CONTENT_TYPE;
use home_orders::repositories::orders::{OrdersRepository, RepositoryError, RepositoryFuture};
use home_orders::server::{AppState, build_router};
use home_orders::services::orders::OrdersService;

/// Serves the configured orders; id lookups filter them.
struct FakeOrdersRepository {
    orders: Vec<Order>,
}

impl OrdersRepository for FakeOrdersRepository {
    fn find_page(&self, _page: u32, _per_page: u32) -> RepositoryFuture<'_, Vec<Order>> {
        let orders = self.orders.clone();
        Box::pin(async move { Ok(orders) })
    }

    fn find_by_id(&self, id: i64) -> RepositoryFuture<'_, Option<Order>> {
        let order = self.orders.iter().find(|order| order.id == id).cloned();
        Box::pin(async move { Ok(order) })
    }

    fn find_by_ids(&self, ids: &[i64]) -> RepositoryFuture<'_, Vec<Order>> {
        let orders = self
            .orders
            .iter()
            .filter(|order| ids.contains(&order.id))
            .cloned()
            .collect();
        Box::pin(async move { Ok(orders) })
    }
}

fn test_order(id: i64) -> Order {
    let created_at = NaiveDateTime::new(
        NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
        NaiveTime::from_hms_micro_opt(12, 0, 0, 123_456).unwrap(),
    );
    Order {
        id,
        user_id: id,
        total: 1_000 * id,
        status: "paid".to_owned(),
        created_at,
    }
}

fn app_with_orders(orders: Vec<Order>) -> TestServer {
    let state = AppState {
        orders: Some(Arc::new(OrdersService::new(Arc::new(
            FakeOrdersRepository { orders },
        )))),
        ..AppState::default()
    };
    TestServer::new(build_router(state))
}

fn content_type(response: &axum_test::TestResponse) -> Option<&str> {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
}

#[tokio::test]
async fn get_orders_returns_the_page_with_camel_case_timestamps() {
    let server = app_with_orders(vec![test_order(1), test_order(2)]);

    let response = server.get("/v1/orders?page=1&perPage=10").await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body = response.json::<serde_json::Value>();
    let orders = body["orders"].as_array().expect("orders must be an array");
    assert_eq!(orders.len(), 2);
    assert_eq!(orders[0]["id"], 1);
    assert_eq!(orders[0]["userId"], 1);
    assert_eq!(orders[0]["total"], 1000);
    assert_eq!(orders[0]["status"], "paid");
    assert_eq!(orders[0]["createdAt"], "2026-11-01T12:00:00.123456");
    assert!(
        orders[0].get("created_at").is_none(),
        "JSON must be camelCase: {body}"
    );
}

#[tokio::test]
async fn get_orders_rejects_page_below_one_with_problem_json_and_errors() {
    let server = app_with_orders(vec![]);

    let response = server.get("/v1/orders?page=0&perPage=10").await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(content_type(&response), Some(PROBLEM_JSON_CONTENT_TYPE));
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["title"], "Bad Request");
    assert_eq!(body["status"], 400);
    let errors = body["errors"].as_array().expect("errors extension");
    assert_eq!(errors[0]["pointer"], "/page");
    assert!(errors[0]["detail"].as_str().is_some_and(|d| !d.is_empty()));
}

#[tokio::test]
async fn get_orders_rejects_missing_query_parameters() {
    let server = app_with_orders(vec![]);

    let response = server.get("/v1/orders").await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(content_type(&response), Some(PROBLEM_JSON_CONTENT_TYPE));
    let body = response.json::<serde_json::Value>();
    let errors = body["errors"].as_array().expect("errors extension");
    assert!(!errors.is_empty());
}

#[tokio::test]
async fn get_orders_reports_service_unavailable_without_a_repository() {
    let server = TestServer::new(build_router(AppState::default()));

    let response = server.get("/v1/orders?page=1&perPage=10").await;

    assert_eq!(response.status_code(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(content_type(&response), Some(PROBLEM_JSON_CONTENT_TYPE));
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["title"], "Service Unavailable");
}

#[tokio::test]
async fn get_order_returns_the_order() {
    let server = app_with_orders(vec![test_order(1), test_order(2)]);

    let response = server.get("/v1/orders/2").await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["id"], 2);
    assert_eq!(body["userId"], 2);
    assert_eq!(body["total"], 2000);
    assert_eq!(body["createdAt"], "2026-11-01T12:00:00.123456");
}

#[tokio::test]
async fn get_order_unknown_id_is_problem_json_404() {
    let server = app_with_orders(vec![test_order(1)]);

    let response = server.get("/v1/orders/99").await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    assert_eq!(content_type(&response), Some(PROBLEM_JSON_CONTENT_TYPE));
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["title"], "Not Found");
    assert_eq!(body["status"], 404);
}

#[tokio::test]
async fn repository_failures_are_problem_json_500() {
    struct FailingRepository;

    impl OrdersRepository for FailingRepository {
        fn find_page(&self, _page: u32, _per_page: u32) -> RepositoryFuture<'_, Vec<Order>> {
            Box::pin(async { Err(RepositoryError::Database(sqlx::Error::RowNotFound)) })
        }

        fn find_by_id(&self, _id: i64) -> RepositoryFuture<'_, Option<Order>> {
            Box::pin(async { Err(RepositoryError::Database(sqlx::Error::RowNotFound)) })
        }

        fn find_by_ids(&self, _ids: &[i64]) -> RepositoryFuture<'_, Vec<Order>> {
            Box::pin(async { Err(RepositoryError::Database(sqlx::Error::RowNotFound)) })
        }
    }

    let state = AppState {
        orders: Some(Arc::new(OrdersService::new(Arc::new(FailingRepository)))),
        ..AppState::default()
    };
    let server = TestServer::new(build_router(state));

    let response = server.get("/v1/orders?page=1&perPage=10").await;
    assert_eq!(response.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(content_type(&response), Some(PROBLEM_JSON_CONTENT_TYPE));
}
