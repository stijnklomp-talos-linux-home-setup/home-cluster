//! Consumer side of the orders Pact contract (ADR-0008).
//!
//! This file is the documentation for the consumer DSL: it states what an
//! "orders client" expects from the service, drives the `pact_consumer` mock
//! server with real requests, and — when the mock server shuts down — writes
//! the contract to `pacts/orders-client-home-orders.json`, which is checked
//! in. A mismatch between the contract and the service fails
//! `tests/pact/provider.rs`.
//!
//! Run both sides with `just test-pact`.

#[path = "common.rs"]
mod common;

use std::path::{Path, PathBuf};

use pact_consumer::mock_server::StartMockServerAsync;
use pact_consumer::prelude::*;
use serde_json::json;

use common::PACT_ORDER_ID;

/// Consumer name: the first half of the pact file name.
const CONSUMER: &str = "orders-client";

/// The checked-in contract: `pacts/<consumer>-<provider>.json`.
fn pact_file() -> PathBuf {
    Path::new("pacts").join(format!("{CONSUMER}-{}.json", home_orders::APP_NAME))
}

#[tokio::test]
async fn orders_client_contract() {
    // The mock server merges into an existing pact file; remove it first so
    // interactions deleted here cannot survive in the checked-in contract.
    let _ = std::fs::remove_file(pact_file());

    let mock = PactBuilderAsync::new_v4(CONSUMER, home_orders::APP_NAME)
        .interaction("a request for a page of orders", "", |mut i| async move {
            // Provider states say what must be true before the request runs.
            i.given("orders exist");
            i.request
                .get()
                .path("/v1/orders")
                .query_param("page", "1")
                .query_param("perPage", "10");
            i.response.ok().json_body(json_pattern!({
                "orders": each_like!({
                    "id": like!(0),
                    "userId": like!(0),
                    "total": like!(0),
                    "status": like!("paid"),
                    "createdAt": like!("2026-11-01T12:00:00.123456"),
                }),
            }));
            i
        })
        .await
        .interaction("a request for one order", "", |mut i| async move {
            i.given("an order exists");
            i.request.get().path(format!("/v1/orders/{PACT_ORDER_ID}"));
            i.response.ok().json_body(json_pattern!({
                "id": PACT_ORDER_ID,
                "userId": like!(0),
                "total": like!(0),
                "status": like!("paid"),
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
        .get(mock.path("/v1/orders?page=1&perPage=10"))
        .send()
        .await
        .expect("GET the mocked orders page");
    assert_eq!(response.status(), 200);
    let page: serde_json::Value = response.json().await.expect("orders page JSON");
    assert_eq!(page["orders"][0]["status"], "paid");

    let response = client
        .get(mock.path(&format!("/v1/orders/{PACT_ORDER_ID}")))
        .send()
        .await
        .expect("GET the mocked order");
    assert_eq!(response.status(), 200);
    let order: serde_json::Value = response.json().await.expect("order JSON");
    assert_eq!(order["id"], json!(PACT_ORDER_ID));

    // Dropping the mock server verifies both interactions and writes
    // `pact_file()`, so the contract is regenerated on every run.
    drop(mock);
}
