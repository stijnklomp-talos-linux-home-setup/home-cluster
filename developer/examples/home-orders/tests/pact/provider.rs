//! Provider side of the orders Pact contract (ADR-0008).
//!
//! Boots the real app on an ephemeral port against the real Postgres from the
//! app's own configuration, then verifies **every checked-in pact in
//! `pacts/`** (drop a new file in and it is picked up). Provider states run
//! through [`PactOrdersStates`]: setup inserts fixture rows, teardown deletes
//! exactly those rows.
//!
//! Needs Postgres on `localhost:5432` (`.env.development` `DATABASE_URL`); run
//! via `just test-pact`.

#[path = "common.rs"]
mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use home_orders::config::AppConfig;
use home_orders::database::Database;
use home_orders::repositories::orders::PgOrdersRepository;
use home_orders::server::{self, AppState};
use home_orders::services::orders::OrdersService;
use home_orders::shutdown;
use pact_models::provider_states::ProviderState;
use pact_verifier::callback_executors::ProviderStateExecutor;
use pact_verifier::{
    FilterInfo, NullRequestFilterExecutor, PactSource, ProviderInfo, ProviderTransport,
    PublishOptions, VerificationOptions, verify_provider_async,
};
use serde_json::Value;
use sqlx::PgPool;

use common::{PACT_ORDER_ID, PACT_SECOND_ORDER_ID, PACT_USER_ID};

#[tokio::test]
async fn verifies_every_checked_in_pact_against_the_real_app() {
    let config = AppConfig::load()
        .expect("app config (run from the repo root; `.env.development` needs DATABASE_URL)");
    let database = Database::connect_lazy(&config.database_url).expect("valid DATABASE_URL");
    let pool = database.pool().clone();

    // Rows a crashed run may have left behind would break the checks below.
    delete_pact_rows(&pool).await.expect("pre-run cleanup");

    // The real app — same router, pool and repository as `main` — on an
    // ephemeral port.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind an ephemeral port");
    let port = listener.local_addr().expect("listener address").port();
    let state = AppState {
        database: Some(database.clone()),
        orders: Some(Arc::new(OrdersService::new(Arc::new(
            PgOrdersRepository::new(pool.clone()),
        )))),
        ..AppState::default()
    };
    let (trigger, watch) = shutdown::channel();
    let app = tokio::spawn(server::serve(
        listener,
        server::build_router(state),
        watch,
        Duration::from_secs(5),
    ));

    let pact_files = pact_files();
    assert!(
        !pact_files.is_empty(),
        "no pacts in ./pacts — `just test-pact` generates the consumer pact first"
    );

    let publish = publish_options();
    let states = Arc::new(PactOrdersStates::new(pool.clone()));
    let result = verify_provider_async(
        provider_info(port),
        pact_sources(&pact_files),
        FilterInfo::None,
        vec![],
        &VerificationOptions {
            coloured_output: false,
            ..VerificationOptions::<NullRequestFilterExecutor>::default()
        },
        publish.as_ref(),
        &states,
        None,
    )
    .await
    .expect("run provider verification");

    delete_pact_rows(&pool).await.expect("post-run cleanup");
    trigger.trigger();
    app.await.expect("server task").expect("clean shutdown");
    database.close().await;

    for file in &pact_files {
        println!("verified {}", file.display());
    }
    for interaction in &result.interaction_results {
        let outcome = if interaction.result.is_ok() {
            "OK"
        } else {
            "FAILED"
        };
        println!("  - {} [{outcome}]", interaction.interaction_description);
    }

    assert!(
        result.result,
        "pact verification failed:\n{}\n{:#?}",
        result.output.join("\n"),
        result.errors
    );
    assert!(
        result.interaction_results.len() >= pact_files.len(),
        "every pact file must contribute at least one interaction"
    );
}

/// Provider-state handlers: a map from state name to real setup/teardown work.
///
/// Setup inserts the fixture rows the contract describes; an unknown state is
/// an error, so a pact that names a state no handler knows cannot pass
/// silently.
#[derive(Debug)]
struct PactOrdersStates {
    pool: PgPool,
}

impl PactOrdersStates {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProviderStateExecutor for PactOrdersStates {
    async fn call(
        self: Arc<Self>,
        _interaction_id: Option<String>,
        provider_state: &ProviderState,
        setup: bool,
        _client: Option<&reqwest::Client>,
    ) -> anyhow::Result<HashMap<String, Value>> {
        if !setup {
            // Teardown runs after every interaction: drop the fixture rows.
            delete_pact_rows(&self.pool).await?;
            return Ok(HashMap::new());
        }

        match provider_state.name.as_str() {
            // Interactions without a `given` still receive an empty state.
            "" => Ok(HashMap::new()),
            "orders exist" => {
                insert_order(&self.pool, PACT_ORDER_ID, 12_500, "paid").await?;
                insert_order(&self.pool, PACT_SECOND_ORDER_ID, 3_400, "shipped").await?;
                Ok(HashMap::new())
            }
            "an order exists" => {
                insert_order(&self.pool, PACT_ORDER_ID, 12_500, "paid").await?;
                Ok(HashMap::new())
            }
            unknown => Err(anyhow::anyhow!(
                "no handler for provider state '{unknown}' — add it to PactOrdersStates"
            )),
        }
    }

    fn teardown(&self) -> bool {
        true
    }
}

/// The provider under verification: the app on the ephemeral port.
fn provider_info(port: u16) -> ProviderInfo {
    #[allow(deprecated)] // the struct still carries legacy host/port fields
    ProviderInfo {
        name: home_orders::APP_NAME.to_owned(),
        host: "127.0.0.1".to_owned(),
        transports: vec![ProviderTransport {
            transport: "http".to_owned(),
            port: Some(port),
            path: None,
            scheme: Some("http".to_owned()),
        }],
        ..ProviderInfo::default()
    }
}

/// Every `pacts/*.json`: dropping a new pact in is picked up automatically.
fn pact_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir("pacts")
        .expect("the `pacts` directory is checked in")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    files
}

/// Where the pacts to verify come from. Broker hook (ADR-0008): the template
/// has no broker, so the checked-in files are the source. A broker deployment
/// replaces this with `PactSource::BrokerUrl` / `BrokerWithDynamicConfiguration`.
fn pact_sources(pact_files: &[PathBuf]) -> Vec<PactSource> {
    pact_files
        .iter()
        .map(|path| PactSource::File(path.display().to_string()))
        .collect()
}

/// Whether verification results are published. Broker hook (ADR-0008): no
/// broker exists, so nothing is published. With broker sources returned by
/// [`pact_sources`], build `PublishOptions { provider_version: Some(<build>),
/// .. }` here and the verifier publishes the results itself.
fn publish_options() -> Option<PublishOptions> {
    None
}

/// Inserts a fixture row with an explicit id, replacing a stale one.
async fn insert_order(pool: &PgPool, id: i64, total: i64, status: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO orders (id, "userId", total, status, "createdAt")
           VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP)
           ON CONFLICT (id) DO UPDATE
             SET "userId" = EXCLUDED."userId",
                 total = EXCLUDED.total,
                 status = EXCLUDED.status"#,
    )
    .bind(id)
    .bind(PACT_USER_ID)
    .bind(total)
    .bind(status)
    .execute(pool)
    .await?;
    Ok(())
}

/// Deletes every row this verification owns. Test-only cleanup, so a plain
/// (non-macro) query keeps the feature-gated pact target out of the committed
/// `.sqlx` offline cache.
async fn delete_pact_rows(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM orders WHERE id >= $1")
        .bind(PACT_ORDER_ID)
        .execute(pool)
        .await?;
    Ok(())
}
