//! HTTP assembly: routes, middleware pipeline, and the serving loop.

use std::future::IntoFuture;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::Request;
use axum::http::{
    HeaderName, HeaderValue, Request as HttpRequest, Response as HttpResponse, StatusCode, header,
};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{
    MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use tower_http::sensitive_headers::SetSensitiveRequestHeadersLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{MakeSpan, OnRequest, OnResponse, TraceLayer};
use tracing::Span;

use crate::database::Database;
use crate::health::ReadinessRegistry;
use crate::infrastructure::cache::CacheClient;
use crate::logging::REQUEST_LOG_TARGET;
use crate::problem::{ApiError, Problem};
use crate::services::orders::OrdersService;
use crate::shutdown::ShutdownWatch;

/// State shared by every handler.
///
/// `main` fills these in before the server accepts traffic; handles are
/// `Option`s where "not configured" is a real state (tests assemble
/// `AppState::default()` without a database). Ticket #5 adds the database pool;
/// ticket #8 adds the AMQP client the same way.
#[derive(Clone, Default)]
pub struct AppState {
    /// Readiness checks run by `GET /readyz`.
    pub readiness: ReadinessRegistry,
    /// Postgres pool; `None` when the app is assembled without a database.
    pub database: Option<Database>,
    /// Cache client shared by services; uninitialized when `CACHE_DISABLED`.
    pub cache: CacheClient,
    /// Orders service; `None` when the app is assembled without a repository.
    pub orders: Option<Arc<OrdersService>>,
    /// Optional GraphQL federation subgraph (feature `graphql`, ADR-0011);
    /// `None` when the feature is compiled out or `GRAPHQL_ENABLED=false`.
    #[cfg(feature = "graphql")]
    pub graphql: Option<crate::graphql::GraphQLState>,
}

impl AppState {
    /// The database handle, or a problem+json 503 when the app was assembled
    /// without one (tests only — `main` always provides it).
    pub fn database(&self) -> Result<&Database, ApiError> {
        self.database
            .as_ref()
            .ok_or_else(|| ApiError::ServiceUnavailable("database is not configured".to_owned()))
    }

    /// The orders service handle, or a problem+json 503 when the app was
    /// assembled without one (tests only — `main` always provides it).
    pub fn orders(&self) -> Result<&OrdersService, ApiError> {
        self.orders
            .as_deref()
            .ok_or_else(|| ApiError::ServiceUnavailable("orders are not configured".to_owned()))
    }

    /// The GraphQL state, or a problem+json 503 when the subgraph is disabled.
    #[cfg(feature = "graphql")]
    pub fn graphql(&self) -> Result<&crate::graphql::GraphQLState, ApiError> {
        self.graphql
            .as_ref()
            .ok_or_else(|| ApiError::ServiceUnavailable("GraphQL is not configured".to_owned()))
    }
}

/// Fastify's default request body limit (1 MiB), kept for parity.
const REQUEST_BODY_LIMIT: usize = 1024 * 1024;

/// Bound on a single request's handler time. Fastify has no request timeout;
/// this is the template's safety net so a stuck handler cannot pin a
/// connection forever (and graceful shutdown always terminates).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Security response headers: the `@fastify/helmet` equivalent (PORTING),
/// helmet's defaults with `crossOriginResourcePolicy: false`. CSP is
/// app-specific and HSTS only means something over TLS, so both are
/// deliberately omitted; a service adds them when it has a browser client or
/// terminates TLS.
const SECURITY_HEADERS: [(&str, &str); 6] = [
    ("x-content-type-options", "nosniff"),
    ("x-frame-options", "SAMEORIGIN"),
    ("referrer-policy", "no-referrer"),
    ("x-dns-prefetch-control", "off"),
    ("x-download-options", "noopen"),
    ("x-permitted-cross-domain-policies", "none"),
];

/// Adds [`SECURITY_HEADERS`] to every response, including pipeline rejections
/// and router fallbacks (this is the outermost layer).
async fn set_security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in SECURITY_HEADERS {
        headers.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    response
}

/// Builds the service app router: routes plus the middleware pipeline.
///
/// Layer order, outermost → innermost: security headers → request-id → trace
/// → sensitive headers → compression → problem+json body-limit rejection →
/// body limit → timeout. (CORS is present but not installed; see
/// [`permissive_cors`].)
pub fn build_router(state: AppState) -> Router {
    let pipeline = ServiceBuilder::new()
        .layer(middleware::from_fn(set_security_headers))
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(RequestSpan)
                .on_request(RequestStarted)
                .on_response(RequestFinished),
        )
        .layer(SetSensitiveRequestHeadersLayer::new([header::AUTHORIZATION]))
        .layer(CompressionLayer::new())
        // CORS: present but off by default — add `.layer(permissive_cors())`
        // (or a stricter `CorsLayer`) to opt in.
        .layer(middleware::from_fn(rewrite_pipeline_rejections))
        .layer(RequestBodyLimitLayer::new(REQUEST_BODY_LIMIT))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ));

    let router = Router::from(crate::routes::app_router());
    // ADR-0011: the GraphQL route is opt-in per build (`graphql` feature) and
    // per environment (`GRAPHQL_ENABLED`), and is not part of the OpenAPI
    // document (which describes the REST contract).
    #[cfg(feature = "graphql")]
    let router = if state.graphql.is_some() {
        router.merge(crate::graphql::routes())
    } else {
        router
    };

    router
        .fallback(crate::problem::not_found)
        .layer(pipeline)
        .with_state(state)
}

/// CORS is **present but off** by default, mirroring the Fastify template
/// (whose `src/middleware/cors.ts` is fully commented out): [`build_router`]
/// installs no CORS middleware at all.
///
/// A service whose clients need CORS opts in explicitly, e.g.
/// `build_router(state).layer(permissive_cors())` (or with a stricter
/// [`CorsLayer`] of its own).
pub fn permissive_cors() -> CorsLayer {
    CorsLayer::permissive()
}

/// Serves `app` until `shutdown` fires, then drains in-flight requests for at
/// most `drain_timeout` before aborting the rest.
///
/// A clean drain returns without a warning; a timeout logs one.
pub async fn serve(
    listener: TcpListener,
    app: Router,
    mut shutdown: ShutdownWatch,
    drain_timeout: Duration,
) -> std::io::Result<()> {
    let mut deadline_watch = shutdown.clone();
    let drain_deadline = async move {
        deadline_watch.wait().await;
        tokio::time::sleep(drain_timeout).await;
    };

    let stop = async move {
        shutdown.wait().await;
    };

    let server = axum::serve(listener, app)
        .with_graceful_shutdown(stop)
        .into_future();

    tokio::pin!(server);
    tokio::pin!(drain_deadline);

    tokio::select! {
        result = &mut server => result?,
        () = &mut drain_deadline => {
            tracing::warn!(
                drain_timeout_ms = drain_timeout.as_millis(),
                "in-flight requests did not finish before the drain timeout; aborting"
            );
        }
    }

    Ok(())
}

/// Problem+json response for the pipeline's own plain/empty rejections:
/// tower-http's body limit (413, text) and timeout (408, empty body) (ADR-0005).
async fn rewrite_pipeline_rejections(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    if is_problem_json(&response) {
        return response;
    }

    let rejection = match response.status() {
        StatusCode::PAYLOAD_TOO_LARGE => Some(
            Problem::new(StatusCode::PAYLOAD_TOO_LARGE).with_detail(format!(
                "request body exceeds the {REQUEST_BODY_LIMIT} byte limit"
            )),
        ),
        StatusCode::REQUEST_TIMEOUT => {
            Some(Problem::new(StatusCode::REQUEST_TIMEOUT).with_detail("request timed out"))
        }
        _ => None,
    };

    match rejection {
        Some(problem) => problem.into_response(),
        None => response,
    }
}

fn is_problem_json(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        == Some(crate::problem::PROBLEM_JSON_CONTENT_TYPE)
}

/// Request log span: carries the fields that identify a request
/// (target: [`REQUEST_LOG_TARGET`]).
#[derive(Clone, Copy)]
struct RequestSpan;

impl<B> MakeSpan<B> for RequestSpan {
    fn make_span(&mut self, request: &HttpRequest<B>) -> Span {
        tracing::info_span!(
            target: REQUEST_LOG_TARGET,
            "request",
            request_id = %request_id(request),
            method = %request.method(),
            path = %request.uri().path(),
        )
    }
}

/// Emits the "started processing request" line.
#[derive(Clone, Copy)]
struct RequestStarted;

impl<B> OnRequest<B> for RequestStarted {
    fn on_request(&mut self, _request: &HttpRequest<B>, span: &Span) {
        tracing::info!(target: REQUEST_LOG_TARGET, parent: span, "started processing request");
    }
}

/// Emits the "finished processing request" line with status and latency.
#[derive(Clone, Copy)]
struct RequestFinished;

impl<B> OnResponse<B> for RequestFinished {
    fn on_response(self, response: &HttpResponse<B>, latency: Duration, span: &Span) {
        tracing::info!(
            target: REQUEST_LOG_TARGET,
            parent: span,
            status = response.status().as_u16(),
            latency_ms = latency.as_secs_f64() * 1000.0,
            "finished processing request"
        );
    }
}

fn request_id<B>(request: &HttpRequest<B>) -> String {
    request
        .extensions()
        .get::<RequestId>()
        .and_then(|id| id.header_value().to_str().ok())
        .unwrap_or("unknown")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use tower::ServiceExt;

    const X_REQUEST_ID: &str = "x-request-id";

    fn app() -> Router {
        build_router(AppState::default())
    }

    async fn response_json(response: Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn healthz_and_readyz_are_204() {
        for path in ["/healthz", "/readyz"] {
            let response = app()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT, "{path}");
        }
    }

    #[test]
    fn app_state_without_a_database_reports_service_unavailable() {
        let error = AppState::default().database().unwrap_err();

        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn unknown_route_is_problem_json_404() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/no-such-route")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            crate::problem::PROBLEM_JSON_CONTENT_TYPE
        );
        let body = response_json(response).await;
        assert_eq!(body["title"], "Not Found");
        assert_eq!(body["status"], 404);
    }

    #[tokio::test]
    async fn responses_carry_a_request_id() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let request_id = response.headers()[X_REQUEST_ID].to_str().unwrap();
        assert!(!request_id.is_empty(), "request id must be set");
    }

    #[tokio::test]
    async fn every_response_carries_the_security_headers() {
        for path in ["/healthz", "/no-such-route"] {
            let response = app()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();

            for (name, expected) in SECURITY_HEADERS {
                assert_eq!(
                    response.headers()[name].to_str().unwrap(),
                    expected,
                    "{path}: {name} missing or wrong"
                );
            }
        }
    }

    #[tokio::test]
    async fn oversized_body_returns_problem_json_413() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/no-such-route")
                    .header(header::CONTENT_LENGTH, REQUEST_BODY_LIMIT + 1)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            crate::problem::PROBLEM_JSON_CONTENT_TYPE
        );
        let body = response_json(response).await;
        assert_eq!(body["status"], 413);
        assert_eq!(body["title"], "Payload Too Large");
    }

    #[tokio::test]
    async fn cors_is_off_by_default_but_opt_in_works() {
        let request = || {
            Request::builder()
                .uri("/healthz")
                .header(header::ORIGIN, "http://example.com")
                .body(Body::empty())
                .unwrap()
        };

        let response = app().oneshot(request()).await.unwrap();
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-origin")
        );

        let response = build_router(AppState::default())
            .layer(permissive_cors())
            .oneshot(request())
            .await
            .unwrap();
        assert_eq!(response.headers()["access-control-allow-origin"], "*");
    }
}
