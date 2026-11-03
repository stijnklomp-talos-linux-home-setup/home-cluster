//! The router's readiness contract against the real database: `main`'s wiring
//! (live `Database` pool registered as the readiness check) answers 204.

use axum::http::StatusCode;
use axum_test::TestServer;

use home_orders::server::{AppState, build_router};

use crate::migrated_database;

#[tokio::test]
async fn readyz_is_204_with_the_real_database_registered() {
    let database = migrated_database().await;

    let mut state = AppState {
        database: Some(database.clone()),
        ..AppState::default()
    };
    state.readiness.register(database.clone());

    let server = TestServer::new(build_router(state));
    let response = server.get("/readyz").await;

    assert_eq!(response.status_code(), StatusCode::NO_CONTENT);
}
