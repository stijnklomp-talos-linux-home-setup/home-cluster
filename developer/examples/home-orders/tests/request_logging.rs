//! Request log capture, in a process of its own.
//!
//! The subscriber is installed as the *global* default so callsite interest is
//! computed against it no matter which test thread registers a callsite first
//! (tracing's interest cache is process-global; thread-local subscribers race
//! with other tests). Each test finds its log lines by request id, so the
//! tests can run in parallel.

use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt;

use home_orders::logging::{REQUEST_LOG_TARGET, build_subscriber};
use home_orders::server::{AppState, build_router};

const X_REQUEST_ID: &str = "x-request-id";

/// Cloneable in-memory log sink.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap_or_default()
    }

    fn lines(&self) -> Vec<serde_json::Value> {
        self.contents()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("log lines are JSON"))
            .collect()
    }
}

impl<'w> fmt::MakeWriter<'w> for Capture {
    type Writer = CaptureWriter;

    fn make_writer(&'w self) -> Self::Writer {
        CaptureWriter(Arc::clone(&self.0))
    }
}

struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl io::Write for CaptureWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Installs the capture subscriber as the process-global default, once.
fn capture() -> &'static Capture {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();
    CAPTURE.get_or_init(|| {
        let capture = Capture::default();
        let subscriber = build_subscriber(LevelFilter::WARN, LevelFilter::INFO, capture.clone());
        tracing::subscriber::set_global_default(subscriber)
            .expect("no other global subscriber in this test binary");
        capture
    })
}

async fn get(path: &str) -> axum::response::Response {
    get_with_headers(path, &[]).await
}

async fn get_with_headers(
    path: &str,
    headers: &[(header::HeaderName, &str)],
) -> axum::response::Response {
    let mut request = Request::builder().uri(path);
    for (name, value) in headers {
        request = request.header(name, *value);
    }
    build_router(AppState::default())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

/// Finds a captured log field by request id; checks the event fields first,
/// then the request span the event belongs to.
fn field<'a>(
    lines: &'a [serde_json::Value],
    request_id: &str,
    name: &str,
) -> Option<&'a serde_json::Value> {
    lines.iter().find_map(|line| {
        line.get("span")
            .and_then(|span| span.get("request_id"))
            .filter(|id| id.as_str() == Some(request_id))?;
        line.get(name)
            .or_else(|| line.get("span").and_then(|span| span.get(name)))
    })
}

#[tokio::test]
async fn request_log_line_carries_id_method_path_status_and_latency() {
    let logs = capture();
    let response = get("/healthz?probe=1").await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let request_id = response.headers()[X_REQUEST_ID]
        .to_str()
        .unwrap()
        .to_owned();

    let lines = logs.lines();
    let finished = lines
        .iter()
        .find(|line| {
            line["target"] == REQUEST_LOG_TARGET
                && line["message"] == "finished processing request"
                && line["span"]["request_id"] == request_id.as_str()
        })
        .unwrap_or_else(|| panic!("no completion line for {request_id}: {lines:#?}"));

    assert_eq!(finished["span"]["method"], "GET");
    assert_eq!(finished["span"]["path"], "/healthz");
    assert_eq!(finished["status"], 204);
    assert!(
        finished["latency_ms"].is_f64(),
        "latency_ms missing: {finished}"
    );
}

#[tokio::test]
async fn started_line_precedes_the_completion_line() {
    let logs = capture();
    let response = get("/healthz").await;
    let request_id = response.headers()[X_REQUEST_ID]
        .to_str()
        .unwrap()
        .to_owned();

    let lines = logs.lines();
    assert!(
        field(&lines, &request_id, "request_id").is_some(),
        "request span missing for {request_id}"
    );

    let messages: Vec<&str> = lines
        .iter()
        .filter(|line| line["span"]["request_id"] == request_id.as_str())
        .filter_map(|line| line["message"].as_str())
        .collect();
    assert_eq!(
        messages,
        vec!["started processing request", "finished processing request"]
    );
}

#[tokio::test]
async fn authorization_header_value_is_never_logged() {
    let logs = capture();
    let response = get_with_headers(
        "/healthz",
        &[(header::AUTHORIZATION, "Bearer super-secret-token")],
    )
    .await;
    let request_id = response.headers()[X_REQUEST_ID]
        .to_str()
        .unwrap()
        .to_owned();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let lines = logs.lines();
    assert!(
        lines.iter().any(|line| {
            line["message"] == "finished processing request"
                && line["span"]["request_id"] == request_id.as_str()
        }),
        "request logging must be on for this test"
    );
    assert!(
        !logs.contents().contains("super-secret-token"),
        "authorization leaked into the logs"
    );
}
