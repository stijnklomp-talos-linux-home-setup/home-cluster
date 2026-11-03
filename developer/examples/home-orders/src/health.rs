//! Health (`/healthz`) and readiness (`/readyz`) endpoints.
//!
//! Readiness goes through [`ReadinessRegistry`]: startup code registers one
//! check per dependency and the registry runs them on every `/readyz` call.
//! `main` registers the live database check via [`ReadinessRegistry::register`].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::problem::{ApiError, Problem};
use crate::server::AppState;

/// Boxed readiness future; the seam every check implements.
pub type ReadinessFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// A dependency readiness probe registered at startup.
pub trait ReadinessCheck: Send + Sync + 'static {
    /// Stable check name, used in logs and the 503 response detail.
    fn name(&self) -> &str;

    /// Returns `Ok(())` when the dependency can serve traffic.
    fn check(&self) -> ReadinessFuture<'_>;
}

/// Readiness failures collected by [`ReadinessRegistry::run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckFailure {
    /// Name of the failing check.
    pub name: String,
    /// Why the check failed.
    pub message: String,
}

/// Registry of readiness checks, owned by [`AppState`].
#[derive(Clone, Default)]
pub struct ReadinessRegistry {
    checks: Vec<Arc<dyn ReadinessCheck>>,
}

impl ReadinessRegistry {
    /// Creates an empty registry: `/readyz` is 204 until a check is registered.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a check (startup only, before the server accepts traffic).
    pub fn register(&mut self, check: impl ReadinessCheck) {
        self.checks.push(Arc::new(check));
    }

    /// Whether no checks are registered.
    pub fn is_empty(&self) -> bool {
        self.checks.is_empty()
    }

    /// Runs every check in registration order; an empty result means ready.
    ///
    /// Checks run sequentially on purpose: readiness is polled rarely and a
    /// handful of checks do not warrant concurrent fan-out yet.
    pub async fn run(&self) -> Vec<CheckFailure> {
        let mut failures = Vec::new();
        for check in &self.checks {
            if let Err(message) = check.check().await {
                failures.push(CheckFailure {
                    name: check.name().to_owned(),
                    message,
                });
            }
        }
        failures
    }
}

/// Routes owned by the health module, merged into
/// [`crate::routes::app_router`] (and documented by the generated OpenAPI).
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(healthz))
        .routes(routes!(readyz))
}

/// Liveness: the process is up. Always 204.
#[utoipa::path(
    get,
    path = "/healthz",
    tag = "health",
    responses((status = 204, description = "The process is up"))
)]
pub async fn healthz() -> StatusCode {
    StatusCode::NO_CONTENT
}

/// Readiness: 204 when every registered check passes, problem+json 503
/// otherwise. Failure messages are logged, not exposed.
#[utoipa::path(
    get,
    path = "/readyz",
    tag = "health",
    responses(
        (status = 204, description = "Every registered check passes"),
        (status = 503, description = "A dependency is unavailable", body = Problem),
    )
)]
pub async fn readyz(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    let failures = state.readiness.run().await;
    if failures.is_empty() {
        return Ok(StatusCode::NO_CONTENT);
    }

    let mut names = Vec::with_capacity(failures.len());
    for failure in &failures {
        tracing::error!(check = %failure.name, error = %failure.message, "readiness check failed");
        names.push(failure.name.as_str());
    }
    Err(ApiError::ServiceUnavailable(format!(
        "readiness check failed: {}",
        names.join(", ")
    )))
}
