//! The users slice against real Postgres: rows written through the repository
//! are read back through the REST endpoints, exactly as `main` wires them.

use std::sync::Arc;

use axum::http::StatusCode;
use axum_test::TestServer;

use home_users::database::Database;
use home_users::repositories::users::PgUsersRepository;
use home_users::server::{AppState, build_router};
use home_users::services::users::UsersService;

use crate::migrated_database;

/// The app as `main` assembles it: database handle, readiness check and the
/// Postgres-backed users service.
fn app(database: &Database) -> TestServer {
    let mut state = AppState {
        database: Some(database.clone()),
        ..AppState::default()
    };
    state.readiness.register(database.clone());
    state.users = Some(Arc::new(UsersService::new(Arc::new(
        PgUsersRepository::new(database.pool().clone()),
    ))));
    TestServer::new(build_router(state))
}

#[tokio::test]
async fn users_flow_round_trips_through_real_postgres() {
    let database = migrated_database().await;
    let repository = PgUsersRepository::new(database.pool().clone());
    let label = crate::unique_label("users");
    let email = format!("{label}@example.test");
    let name = format!("Acceptance {label}");

    let inserted = repository
        .insert(&name, &email)
        .await
        .expect("insert the acceptance user");

    let server = app(&database);
    let response = server.get("/v1/users?page=1&perPage=100").await;
    assert_eq!(response.status_code(), StatusCode::OK);
    let body = response.json::<serde_json::Value>();
    let users = body["users"].as_array().expect("users must be an array");
    let found = users
        .iter()
        .find(|user| user["id"].as_i64() == Some(inserted.id))
        .expect("the inserted user must be on the first page");
    assert_eq!(found["email"], email);
    assert_eq!(found["name"], name);

    let response = server.get(&format!("/v1/users/{}", inserted.id)).await;
    assert_eq!(response.status_code(), StatusCode::OK);
    let fetched = response.json::<serde_json::Value>();
    assert_eq!(fetched["id"], inserted.id);
    assert_eq!(fetched["email"], email);
    assert_eq!(fetched["createdAt"], found["createdAt"]);
}

#[tokio::test]
async fn unknown_user_is_problem_json_404_through_the_real_router() {
    let server = app(&migrated_database().await);

    let response = server.get("/v1/users/9223372036854775807").await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some(home_users::problem::PROBLEM_JSON_CONTENT_TYPE)
    );
}
