//! JSON logs on stdout with two independent filters (ADR-0007).
//!
//! Two [`tracing_subscriber`] layers share the process:
//!
//! - the **request layer** enables exactly the [`REQUEST_LOG_TARGET`] target at
//!   `REQUEST_LOG_LEVEL`;
//! - the **app layer** enables every other target at `APP_LOG_LEVEL` and turns
//!   [`REQUEST_LOG_TARGET`] off.
//!
//! The filters can never overlap, so changing one level cannot change the
//! other stream's verbosity. Sensitive request headers (the `authorization`
//! header) are never logged: the trace layer only emits explicitly listed
//! fields (request id, method, path, status, latency) and
//! `SetSensitiveRequestHeadersLayer` additionally redacts them in any debug
//! output of request headers.

use std::io;

use tracing::Subscriber;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, Registry};

use crate::config::AppConfig;

/// Tracing target of HTTP request logs.
pub const REQUEST_LOG_TARGET: &str = "request";

/// Installs the global subscriber: JSON on stdout, identical in every
/// environment (ADR-0007).
pub fn init(config: &AppConfig) -> Result<(), LoggingError> {
    build_subscriber(config.app_log_level, config.request_log_level, io::stdout)
        .try_init()
        .map_err(LoggingError)
}

/// Error installing the global subscriber.
#[derive(Debug, thiserror::Error)]
#[error("failed to install the global tracing subscriber: {0}")]
pub struct LoggingError(#[from] tracing_subscriber::util::TryInitError);

/// Builds the subscriber without installing it; used by `tests/request_logging.rs`
/// (which installs it globally so it owns the test process) and by in-crate
/// capture tests.
pub fn build_subscriber<W>(
    app_level: LevelFilter,
    request_level: LevelFilter,
    writer: W,
) -> impl Subscriber + Send + Sync
where
    W: for<'w> fmt::MakeWriter<'w> + Clone + Send + Sync + 'static,
{
    let app_layer = json_layer(writer.clone()).with_filter(
        Targets::new()
            .with_default(app_level)
            .with_target(REQUEST_LOG_TARGET, LevelFilter::OFF),
    );
    let request_layer = json_layer(writer).with_filter(
        Targets::new()
            .with_default(LevelFilter::OFF)
            .with_target(REQUEST_LOG_TARGET, request_level),
    );

    Registry::default().with(app_layer).with(request_layer)
}

fn json_layer<S, W>(
    writer: W,
) -> fmt::Layer<S, fmt::format::JsonFields, fmt::format::Format<fmt::format::Json>, W>
where
    S: Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
    W: for<'w> fmt::MakeWriter<'w> + Clone + Send + Sync + 'static,
{
    fmt::layer()
        .json()
        .flatten_event(true)
        .with_ansi(false)
        .with_target(true)
        .with_file(false)
        .with_line_number(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_writer(writer)
}

/// In-memory log capture used by the crate's tests.
#[cfg(test)]
pub(crate) mod test_support {
    use std::io;
    use std::sync::{Arc, Mutex};

    use tracing::level_filters::LevelFilter;

    /// Cloneable in-memory writer for [`super::build_subscriber`].
    #[derive(Clone, Default)]
    pub(crate) struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// All captured output as text.
        pub(crate) fn contents(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap_or_default()
        }

        /// Captured lines parsed as JSON objects.
        pub(crate) fn lines(&self) -> Vec<serde_json::Value> {
            self.contents()
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| serde_json::from_str(line).expect("log lines are JSON objects"))
                .collect()
        }
    }

    impl<'w> tracing_subscriber::fmt::MakeWriter<'w> for Capture {
        type Writer = CaptureWriter;

        fn make_writer(&'w self) -> Self::Writer {
            CaptureWriter(Arc::clone(&self.0))
        }
    }

    /// Writer handed out by [`Capture`].
    pub(crate) struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl io::Write for CaptureWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    static CAPTURE_LOCK: Mutex<()> = Mutex::new(());

    /// Runs `f` with a capture subscriber installed on this thread; returns the
    /// result and the captured output.
    ///
    /// Capture scopes are serialized and rebuild the global callsite interest
    /// cache: a callsite first used without a subscriber caches `never` for the
    /// whole process, so otherwise parallel tests would poison each other.
    pub(crate) fn run_capture<T>(
        app_level: LevelFilter,
        request_level: LevelFilter,
        f: impl FnOnce() -> T,
    ) -> (T, Capture) {
        let _serialized = CAPTURE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let capture = Capture::new();
        let subscriber = super::build_subscriber(app_level, request_level, capture.clone());
        let result = tracing::subscriber::with_default(subscriber, || {
            tracing_core::callsite::rebuild_interest_cache();
            f()
        });
        (result, capture)
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::run_capture;
    use super::*;

    #[test]
    fn request_level_does_not_affect_app_logs() {
        let ((), capture) = run_capture(LevelFilter::WARN, LevelFilter::DEBUG, || {
            tracing::info!(target: REQUEST_LOG_TARGET, "request line");
            tracing::info!(target: "app", "app line");
        });

        let output = capture.contents();
        assert!(output.contains("request line"), "request logs: {output}");
        assert!(
            !output.contains("app line"),
            "app logs must stay at warn: {output}"
        );
    }

    #[test]
    fn app_level_does_not_affect_request_logs() {
        let ((), capture) = run_capture(LevelFilter::DEBUG, LevelFilter::WARN, || {
            tracing::info!(target: REQUEST_LOG_TARGET, "request line");
            tracing::info!(target: "app", "app line");
        });

        let output = capture.contents();
        assert!(
            !output.contains("request line"),
            "request logs must stay at warn: {output}"
        );
        assert!(output.contains("app line"), "app logs: {output}");
    }

    #[test]
    fn logs_are_json_objects_one_per_line() {
        let ((), capture) = run_capture(LevelFilter::INFO, LevelFilter::INFO, || {
            tracing::info!(target: REQUEST_LOG_TARGET, "request line");
            tracing::warn!(target: "app", count = 3, "app line");
        });

        let lines = capture.lines();
        assert_eq!(lines.len(), 2, "{lines:#?}");
        assert!(lines.iter().all(|line| line["level"].is_string()));
        assert!(
            lines
                .iter()
                .any(|line| line["target"] == REQUEST_LOG_TARGET)
        );
    }

    #[test]
    fn either_stream_can_be_switched_off() {
        let ((), capture) = run_capture(LevelFilter::OFF, LevelFilter::OFF, || {
            tracing::info!(target: REQUEST_LOG_TARGET, "request line");
            tracing::info!(target: "app", "app line");
        });

        assert_eq!(capture.contents().trim(), "");
    }
}
