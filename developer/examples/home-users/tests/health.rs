//! Health and readiness behavior through the public router.

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

use home_users::database::Database;
use home_users::health::{ReadinessCheck, ReadinessFuture, ReadinessRegistry};
use home_users::problem::PROBLEM_JSON_CONTENT_TYPE;
use home_users::server::{AppState, build_router};

fn app(state: AppState) -> Router {
    build_router(state)
}

#[tokio::test]
async fn healthz_is_204() {
    let response = app(AppState::default())
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn readyz_is_204_without_registered_checks() {
    let response = app(AppState::default())
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

struct FailingCheck;

impl ReadinessCheck for FailingCheck {
    fn name(&self) -> &str {
        "database"
    }

    fn check(&self) -> ReadinessFuture<'_> {
        Box::pin(async { Err("connection refused".to_owned()) })
    }
}

struct PassingCheck;

impl ReadinessCheck for PassingCheck {
    fn name(&self) -> &str {
        "cache"
    }

    fn check(&self) -> ReadinessFuture<'_> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn readyz_is_problem_json_503_when_a_check_fails() {
    let mut state = AppState::default();
    state.readiness.register(PassingCheck);
    state.readiness.register(FailingCheck);

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        PROBLEM_JSON_CONTENT_TYPE
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("database"), "{body}");
    assert!(
        !body.contains("connection refused"),
        "check messages must not leak into responses: {body}"
    );
}

#[tokio::test]
async fn presenting_readyz_registry_shape_to_later_tickets() {
    // Ticket #5 registers the database check in exactly this shape.
    let registry: ReadinessRegistry = {
        let mut registry = ReadinessRegistry::new();
        registry.register(PassingCheck);
        registry
    };
    assert!(!registry.is_empty());
    assert!(registry.run().await.is_empty());
}

/// The acceptance path for a downed database, minus the live server: the real
/// router, the real [`Database`] check, a URL that cannot connect.
#[tokio::test]
async fn readyz_is_problem_json_503_when_the_database_is_unreachable() {
    let database = Database::connect_lazy("postgres://user:pass@127.0.0.1:1/db").unwrap();

    let mut state = AppState::default();
    state.readiness.register(database.clone());
    state.database = Some(database);

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        PROBLEM_JSON_CONTENT_TYPE
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["title"], "Service Unavailable");
    assert_eq!(body["status"], 503);
    assert_eq!(body["detail"], "readiness check failed: database");
}

#[tokio::test]
async fn unknown_route_is_problem_json_404() {
    let response = app(AppState::default())
        .oneshot(
            Request::builder()
                .uri("/definitely-not-here")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        PROBLEM_JSON_CONTENT_TYPE
    );

    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["title"], "Not Found");
    assert_eq!(body["status"], 404);
    assert_eq!(body["type"], "about:blank");
}
