//! Consumer side of the users Pact contract (ADR-0008).
//!
//! This file is the documentation for the consumer DSL: it states what a
//! "users client" expects from the service, drives the `pact_consumer` mock
//! server with real requests, and — when the mock server shuts down — writes
//! the contract to `pacts/users-client-home-users.json`, which is checked in.
//! A mismatch between the contract and the service fails
//! `tests/pact/provider.rs`.
//!
//! Run both sides with `just test-pact`.

#[path = "common.rs"]
mod common;

use std::path::{Path, PathBuf};

use pact_consumer::mock_server::StartMockServerAsync;
use pact_consumer::prelude::*;
use serde_json::json;

use common::PACT_USER_ID;

/// Consumer name: the first half of the pact file name.
const CONSUMER: &str = "users-client";

/// The checked-in contract: `pacts/<consumer>-<provider>.json`.
fn pact_file() -> PathBuf {
    Path::new("pacts").join(format!("{CONSUMER}-{}.json", home_users::APP_NAME))
}

#[tokio::test]
async fn users_client_contract() {
    // The mock server merges into an existing pact file; remove it first so
    // interactions deleted here cannot survive in the checked-in contract.
    let _ = std::fs::remove_file(pact_file());

    let mock = PactBuilderAsync::new_v4(CONSUMER, home_users::APP_NAME)
        .interaction("a request for a page of users", "", |mut i| async move {
            // Provider states say what must be true before the request runs.
            i.given("users exist");
            i.request
                .get()
                .path("/v1/users")
                .query_param("page", "1")
                .query_param("perPage", "10");
            i.response.ok().json_body(json_pattern!({
                "users": each_like!({
                    "id": like!(0),
                    "email": like!("ada@example.test"),
                    "name": like!("Ada Lovelace"),
                    "createdAt": like!("2026-11-01T12:00:00.123456"),
                }),
            }));
            i
        })
        .await
        .interaction("a request for one user", "", |mut i| async move {
            i.given("a user exists");
            i.request.get().path(format!("/v1/users/{PACT_USER_ID}"));
            i.response.ok().json_body(json_pattern!({
                "id": PACT_USER_ID,
                "email": like!("ada@example.test"),
                "name": like!("Ada Lovelace"),
                "createdAt": like!("2026-11-01T12:00:00.123456"),
            }));
            i
        })
        .await
        .output_dir("pacts")
        .start_mock_server_async(None, None)
        .await;

    // A stand-in for the real client: exercising the mock server is what
    // proves these are the requests the contract describes.
    let client = reqwest::Client::new();

    let response = client
        .get(mock.path("/v1/users?page=1&perPage=10"))
        .send()
        .await
        .expect("GET the mocked users page");
    assert_eq!(response.status(), 200);
    let page: serde_json::Value = response.json().await.expect("users page JSON");
    assert_eq!(page["users"][0]["email"], "ada@example.test");

    let response = client
        .get(mock.path(&format!("/v1/users/{PACT_USER_ID}")))
        .send()
        .await
        .expect("GET the mocked user");
    assert_eq!(response.status(), 200);
    let user: serde_json::Value = response.json().await.expect("user JSON");
    assert_eq!(user["id"], json!(PACT_USER_ID));

    // Dropping the mock server verifies both interactions and writes
    // `pact_file()`, so the contract is regenerated on every run.
    drop(mock);
}
