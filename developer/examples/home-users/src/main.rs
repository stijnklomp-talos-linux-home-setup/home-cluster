//! Service app entrypoint: load config, install logging, build the router,
//! serve until a shutdown signal arrives, then run cleanup.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use home_users::amqp;
use home_users::config::AppConfig;
use home_users::database::Database;
use home_users::infrastructure::cache::CacheClient;
use home_users::openapi;
use home_users::repositories::users::PgUsersRepository;
use home_users::server::{self, AppState};
use home_users::services::users::UsersService;
use home_users::shutdown;
use home_users::{APP_NAME, APP_VERSION, logging};

#[tokio::main]
async fn main() -> ExitCode {
    let config = match AppConfig::load() {
        Ok(config) => config,
        Err(error) => {
            // Logging is not installed yet, so startup errors go to stderr.
            eprintln!("failed to load configuration: {error}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = logging::init(&config) {
        eprintln!("failed to initialize logging: {error}");
        return ExitCode::FAILURE;
    }

    tracing::info!(
        app = APP_NAME,
        version = APP_VERSION,
        app_env = config.app_env.as_str(),
        port = config.api_port,
        "service app starting"
    );

    // Infrastructure clients connect before the server accepts traffic
    // (Fastify parity: `cacheClient.init()` precedes `app.listen`) and a
    // connection failure is a startup error. The database pool connects lazily:
    // the service starts (and `/readyz` reports 503) while Postgres is down.
    let mut cache = match CacheClient::connect(&config.cache).await {
        Ok(cache) => cache,
        Err(error) => {
            tracing::error!(error = %error, "failed to initialize the cache client");
            return ExitCode::FAILURE;
        }
    };

    let database = match Database::connect_lazy(&config.database_url) {
        Ok(database) => database,
        Err(error) => {
            tracing::error!(error = %error, "invalid DATABASE_URL");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!("database pool configured");

    let mut state = AppState {
        cache: cache.clone(),
        ..AppState::default()
    };
    state.readiness.register(database.clone());
    state.database = Some(database.clone());
    let users = Arc::new(UsersService::new(Arc::new(PgUsersRepository::new(
        database.pool().clone(),
    ))));
    state.users = Some(users.clone());

    #[cfg(feature = "graphql")]
    if config.graphql.enabled {
        state.graphql = Some(home_users::graphql::GraphQLState::new(
            &config.graphql,
            users,
        ));
        tracing::info!("GraphQL federation subgraph enabled at /graphql");
    }

    let router = server::build_router(state);

    // ADR-0004: the OpenAPI document is generated from the code and written on
    // non-production startup (Fastify parity); a write failure is a startup
    // error, like every other startup step.
    match openapi::write_on_startup(config.app_env, Path::new(openapi::DEFAULT_PATH)) {
        Ok(true) => tracing::info!(path = openapi::DEFAULT_PATH, "OpenAPI document written"),
        Ok(false) => {}
        Err(error) => {
            tracing::error!(error = %error, "failed to write the OpenAPI document");
            return ExitCode::FAILURE;
        }
    }

    let listener = match tokio::net::TcpListener::bind(("0.0.0.0", config.api_port)).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(port = config.api_port, error = %error, "failed to bind the listener");
            return ExitCode::FAILURE;
        }
    };
    match listener.local_addr() {
        Ok(address) => tracing::info!(address = %address, "listening"),
        Err(error) => tracing::warn!(error = %error, "listening, but the address is unknown"),
    }

    let (trigger, watch) = shutdown::channel();
    tokio::spawn(async move {
        shutdown::wait_for_signal().await;
        tracing::info!("shutdown signal received; draining");
        trigger.trigger();
    });

    // Cleanup runs in registration order: database pool, cache, then AMQP.
    let mut cleanup = shutdown::CleanupRegistry::new();
    cleanup.register("database", async move { database.close().await });
    // The cache closes when this last handle drops.
    cleanup.register("cache", async move { cache.close() });

    // Ticket #8: RabbitMQ publisher, skipped entirely (no connection attempt)
    // when `RABBIT_DISABLED` disables it.
    if config.rabbit.enabled {
        match amqp::connect_and_declare(&config.rabbit).await {
            Ok(client) => {
                tracing::info!(
                    host = %config.rabbit.host,
                    port = config.rabbit.port,
                    "RabbitMQ connected"
                );
                cleanup.register("amqp", async move {
                    if let Err(error) = client.close().await {
                        tracing::warn!(error = %error, "failed to close the RabbitMQ connection");
                    } else {
                        tracing::info!("RabbitMQ connection closed");
                    }
                });
            }
            Err(error) => {
                tracing::error!(error = %error, "failed to initialize RabbitMQ");
                return ExitCode::FAILURE;
            }
        }
    }

    let serve_result = server::serve(listener, router, watch, shutdown::DRAIN_TIMEOUT).await;
    cleanup.run_all(shutdown::CLEANUP_TIMEOUT).await;

    match serve_result {
        Ok(()) => {
            tracing::info!("shutdown complete");
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!(error = %error, "server stopped with an error");
            ExitCode::FAILURE
        }
    }
}
