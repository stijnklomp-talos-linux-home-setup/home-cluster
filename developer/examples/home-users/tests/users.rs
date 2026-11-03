//! Feature tests for the users REST slice: the endpoints through the real
//! router, driven in-process by axum-test, with a hand-written fake repository
//! behind the service (the house idiom — no database).

use std::sync::Arc;

use axum::http::{StatusCode, header};
use axum_test::TestServer;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use home_users::models::user::User;
use home_users::problem::PROBLEM_JSON_CONTENT_TYPE;
use home_users::repositories::users::{RepositoryError, RepositoryFuture, UsersRepository};
use home_users::server::{AppState, build_router};
use home_users::services::users::UsersService;

/// Serves the configured users; id lookups filter them.
struct FakeUsersRepository {
    users: Vec<User>,
}

impl UsersRepository for FakeUsersRepository {
    fn find_page(&self, _page: u32, _per_page: u32) -> RepositoryFuture<'_, Vec<User>> {
        let users = self.users.clone();
        Box::pin(async move { Ok(users) })
    }

    fn find_by_id(&self, id: i64) -> RepositoryFuture<'_, Option<User>> {
        let user = self.users.iter().find(|user| user.id == id).cloned();
        Box::pin(async move { Ok(user) })
    }

    fn find_by_ids(&self, ids: &[i64]) -> RepositoryFuture<'_, Vec<User>> {
        let users = self
            .users
            .iter()
            .filter(|user| ids.contains(&user.id))
            .cloned()
            .collect();
        Box::pin(async move { Ok(users) })
    }
}

fn test_user(id: i64) -> User {
    let created_at = NaiveDateTime::new(
        NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
        NaiveTime::from_hms_micro_opt(12, 0, 0, 123_456).unwrap(),
    );
    User {
        id,
        email: format!("user{id}@example.test"),
        name: format!("User {id}"),
        created_at,
    }
}

fn app_with_users(users: Vec<User>) -> TestServer {
    let state = AppState {
        users: Some(Arc::new(UsersService::new(Arc::new(FakeUsersRepository {
            users,
        })))),
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
async fn get_users_returns_the_page_with_camel_case_timestamps() {
    let server = app_with_users(vec![test_user(1), test_user(2)]);

    let response = server.get("/v1/users?page=1&perPage=10").await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body = response.json::<serde_json::Value>();
    let users = body["users"].as_array().expect("users must be an array");
    assert_eq!(users.len(), 2);
    assert_eq!(users[0]["id"], 1);
    assert_eq!(users[0]["email"], "user1@example.test");
    assert_eq!(users[0]["name"], "User 1");
    assert_eq!(users[0]["createdAt"], "2026-11-01T12:00:00.123456");
    assert!(
        users[0].get("created_at").is_none(),
        "JSON must be camelCase: {body}"
    );
}

#[tokio::test]
async fn get_users_rejects_page_below_one_with_problem_json_and_errors() {
    let server = app_with_users(vec![]);

    let response = server.get("/v1/users?page=0&perPage=10").await;

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
async fn get_users_rejects_missing_query_parameters() {
    let server = app_with_users(vec![]);

    let response = server.get("/v1/users").await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(content_type(&response), Some(PROBLEM_JSON_CONTENT_TYPE));
    let body = response.json::<serde_json::Value>();
    let errors = body["errors"].as_array().expect("errors extension");
    assert!(!errors.is_empty());
}

#[tokio::test]
async fn get_users_reports_service_unavailable_without_a_repository() {
    let server = TestServer::new(build_router(AppState::default()));

    let response = server.get("/v1/users?page=1&perPage=10").await;

    assert_eq!(response.status_code(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(content_type(&response), Some(PROBLEM_JSON_CONTENT_TYPE));
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["title"], "Service Unavailable");
}

#[tokio::test]
async fn get_user_returns_the_user() {
    let server = app_with_users(vec![test_user(1), test_user(2)]);

    let response = server.get("/v1/users/2").await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["id"], 2);
    assert_eq!(body["name"], "User 2");
    assert_eq!(body["createdAt"], "2026-11-01T12:00:00.123456");
}

#[tokio::test]
async fn get_user_unknown_id_is_problem_json_404() {
    let server = app_with_users(vec![test_user(1)]);

    let response = server.get("/v1/users/99").await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
    assert_eq!(content_type(&response), Some(PROBLEM_JSON_CONTENT_TYPE));
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["title"], "Not Found");
    assert_eq!(body["status"], 404);
}

#[tokio::test]
async fn repository_failures_are_problem_json_500() {
    struct FailingRepository;

    impl UsersRepository for FailingRepository {
        fn find_page(&self, _page: u32, _per_page: u32) -> RepositoryFuture<'_, Vec<User>> {
            Box::pin(async { Err(RepositoryError::Database(sqlx::Error::RowNotFound)) })
        }

        fn find_by_id(&self, _id: i64) -> RepositoryFuture<'_, Option<User>> {
            Box::pin(async { Err(RepositoryError::Database(sqlx::Error::RowNotFound)) })
        }

        fn find_by_ids(&self, _ids: &[i64]) -> RepositoryFuture<'_, Vec<User>> {
            Box::pin(async { Err(RepositoryError::Database(sqlx::Error::RowNotFound)) })
        }
    }

    let state = AppState {
        users: Some(Arc::new(UsersService::new(Arc::new(FailingRepository)))),
        ..AppState::default()
    };
    let server = TestServer::new(build_router(state));

    let response = server.get("/v1/users?page=1&perPage=10").await;
    assert_eq!(response.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(content_type(&response), Some(PROBLEM_JSON_CONTENT_TYPE));
}
