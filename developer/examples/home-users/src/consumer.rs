//! AMQP consumer: a manual-ack consume loop with the frozen failure path
//! (ADR-0006) and the dedicated-worker recipe (ADR-0002).
//!
//! # Consuming
//!
//! [`Consumer::start`] opens its own channel on the client's connection,
//! applies the prefetch limit, declares the configured topology and starts
//! consuming with manual acknowledgements. For every delivery:
//!
//! 1. a body that is not valid UTF-8 is unparseable: the handler is not
//!    called and the failure path below records `originalContent` as
//!    `"UNPARSEABLE"`;
//! 2. the handler runs with the decoded body;
//! 3. on success the delivery is acked;
//! 4. on failure the error is republished to `deadLetter` with routing key
//!    `<queue>.failed` and the frozen payload (`error`, `errorName`,
//!    `failedAt`, `originalContent`, `queueName`) **before** the delivery is
//!    acked — never nacked, never requeued. A failed DLQ publish is logged and
//!    the ack still happens (the Fastify convention: log and continue).
//!
//! The subscription comes from `WORKER_QUEUE` (default `task_queue`),
//! `WORKER_EXCHANGE` (default `main`), `WORKER_BINDING_KEY` (default `task.#`)
//! and `WORKER_PREFETCH` (default 1; `0` means unlimited): all four are part
//! of [`crate::config::WorkerConfig`]. [`DemoTaskHandler`] is the example
//! handler (marked for removal) — a real service writes its own
//! [`TaskHandler`].
//!
//! # Dedicated worker deployments
//!
//! ADR-0002: the service app is the only shipped binary and it does not start
//! consumers. A service that wants a dedicated consumer deployment adds
//! `src/bin/worker.rs` (per-service, not part of the template):
//!
//! ```rust,no_run
//! use std::process::ExitCode;
//!
//! use home_users::config::AppConfig;
//! use home_users::consumer::{Consumer, DemoTaskHandler};
//! use home_users::shutdown;
//! use home_users::{amqp, logging};
//!
//! #[tokio::main]
//! async fn main() -> Result<ExitCode, Box<dyn std::error::Error>> {
//!     let config = AppConfig::load()?;
//!     logging::init(&config)?;
//!
//!     let client = amqp::connect_and_declare(&config.rabbit).await?;
//!     let mut consumer = Consumer::start(&client, &config.worker, DemoTaskHandler).await?;
//!
//!     let (trigger, mut watch) = shutdown::channel();
//!     tokio::spawn(async move {
//!         shutdown::wait_for_signal().await;
//!         trigger.trigger();
//!     });
//!
//!     tokio::select! {
//!         _ = watch.wait() => {}
//!         stopped = consumer.wait_for_stop() => {
//!             // The broker cancelled the consumer (e.g. its queue was
//!             // deleted). A dedicated worker has no other purpose: stop.
//!             stopped?;
//!         }
//!     }
//!
//!     // Order matters: cancel the consumer and close its channel first, then
//!     // the connection. Shutdown is idempotent, so it is safe on both paths.
//!     consumer.shutdown().await?;
//!     client.close().await?;
//!     Ok(ExitCode::SUCCESS)
//! }
//! ```
//!
//! Wiring the consumer **in-process** instead: register its shutdown in the
//! `CleanupRegistry` *before* the AMQP client's cleanup, so the cancel →
//! drain → close-channel order (research §5) is preserved; then the
//! connection is closed by the client's own hook.
//!
//! A panicking handler is logged and its delivery is left unacked; the
//! broker requeues it when the channel closes.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, SecondsFormat, Utc};
use futures_lite::StreamExt;
use lapin::message::Delivery;
use lapin::options::{
    BasicAckOptions, BasicCancelOptions, BasicConsumeOptions, BasicQosOptions,
    ExchangeDeclareOptions, QueueBindOptions, QueueDeclareOptions,
};
use lapin::types::FieldTable;
use lapin::{Channel, Consumer as BrokerConsumer, ExchangeKind};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::task::{JoinError, JoinHandle, JoinSet};

use crate::amqp::{
    AmqpClient, DEAD_LETTER_EXCHANGE, Publisher, exchange_arguments, queue_arguments,
};
use crate::config::WorkerConfig;
use crate::shutdown::{self, ShutdownTrigger, ShutdownWatch};

/// Recorded as `originalContent` when a body is not valid UTF-8 (frozen
/// Fastify parity).
pub const UNPARSEABLE: &str = "UNPARSEABLE";

/// The frozen DLQ envelope (ADR-0006), keyed exactly like the Fastify
/// template's `JSON.stringify({...})`.
///
/// `failedAt` is ISO-8601 UTC with milliseconds (`toISOString()` parity);
/// `originalContent` is the decoded body, or `"UNPARSEABLE"` when the body
/// was not valid UTF-8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadLetterPayload {
    /// The handler failure's message (`err.message` parity).
    pub error: String,
    /// The handler failure's name (`err.name` parity).
    pub error_name: String,
    /// When the failure happened (`new Date().toISOString()` parity).
    pub failed_at: String,
    /// The delivery's body, or `"UNPARSEABLE"`.
    pub original_content: String,
    /// The consumed queue's name; the routing key is `<queueName>.failed`.
    pub queue_name: String,
}

impl DeadLetterPayload {
    /// Builds the envelope for a handler failure, stamping `failedAt` now.
    fn for_failure(error: &TaskError, original_content: &str, queue_name: &str) -> Self {
        Self {
            error: error.message().to_owned(),
            error_name: error.name().to_owned(),
            failed_at: format_iso8601(Utc::now()),
            original_content: original_content.to_owned(),
            queue_name: queue_name.to_owned(),
        }
    }
}

/// Formats `timestamp` like JS `new Date().toISOString()`: ISO-8601 UTC with
/// exactly three fractional digits (`2026-10-16T09:41:07.123Z`).
fn format_iso8601(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Why a task handler failed, carrying the two fields the frozen DLQ payload
/// records (`errorName`, `error`).
///
/// The Fastify code read `err.name` and `err.message` off a thrown `Error`
/// and fell back to `"Unknown"`/`String(err)` otherwise; in Rust every failure
/// states both explicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskError {
    name: String,
    message: String,
}

impl TaskError {
    /// Creates a failure with an explicit name and message.
    pub fn new(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            message: message.into(),
        }
    }

    /// The failure name, recorded as `errorName` (`err.name` parity).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The failure message, recorded as `error` (`err.message` parity).
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for TaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TaskError {}

/// Processes one message body.
///
/// `Err` triggers the frozen failure path: republish to `deadLetter`, then
/// ack ([`Consumer`] module docs).
pub trait TaskHandler: Send + Sync + 'static {
    /// Handles the decoded body of one delivery.
    fn handle(&self, task: &str) -> impl Future<Output = Result<(), TaskError>> + Send;
}

/// A consumer failed to start, or its consume loop failed.
#[derive(Debug, Error)]
pub enum ConsumerError {
    /// Opening the consumer channel failed.
    #[error("failed to open the consumer channel: {source}")]
    OpenChannel {
        /// Underlying lapin error (boxed: it dwarfs every other field).
        #[source]
        source: Box<lapin::Error>,
    },
    /// Applying the prefetch limit failed.
    #[error("failed to set the consumer prefetch to {prefetch}: {source}")]
    Prefetch {
        /// Configured `WORKER_PREFETCH`.
        prefetch: u16,
        /// Underlying lapin error (boxed: it dwarfs every other field).
        #[source]
        source: Box<lapin::Error>,
    },
    /// Declaring the consumer's exchange, queue or binding failed.
    #[error("failed to declare {kind} '{name}': {source}")]
    Declare {
        /// What was declared (`exchange`, `queue` or `binding`).
        kind: &'static str,
        /// Declared subject (name or binding key).
        name: String,
        /// Underlying lapin error (boxed: it dwarfs every other field).
        #[source]
        source: Box<lapin::Error>,
    },
    /// Starting the consumer subscription failed.
    #[error("failed to start consuming from queue '{queue}': {source}")]
    Consume {
        /// Queue the consumer subscribes to.
        queue: String,
        /// Underlying lapin error (boxed: it dwarfs every other field).
        #[source]
        source: Box<lapin::Error>,
    },
    /// The consumer stream ended with an error.
    #[error("the consumer stream ended with an error: {source}")]
    Stream {
        /// Underlying lapin error (boxed: it dwarfs every other field).
        #[source]
        source: Box<lapin::Error>,
    },
    /// The consumer's background task failed.
    #[error("the consumer task failed: {source}")]
    Task {
        /// Task join error (the loop task panicked or was cancelled).
        #[source]
        source: JoinError,
    },
}

/// A running consumer deployment: the broker-side subscription plus the
/// background loop draining deliveries.
///
/// Dropping the handle without calling [`Consumer::shutdown`] still stops the
/// loop (it observes the dropped trigger and cancels on its own), but only
/// `shutdown` waits for the cancel → drain → close sequence to finish.
pub struct Consumer {
    shutdown: Option<ShutdownTrigger>,
    loop_task: Option<JoinHandle<Result<(), ConsumerError>>>,
}

impl Consumer {
    /// Opens the consumer channel, applies `WORKER_PREFETCH`, declares the
    /// `WORKER_QUEUE`/`WORKER_EXCHANGE`/`WORKER_BINDING_KEY` topology and
    /// starts consuming with manual acknowledgements.
    ///
    /// The queue is declared with the frozen arguments (1h TTL, `deadLetter`
    /// DLX) and kept durable, like every queue the TS wrapper declares.
    pub async fn start(
        client: &AmqpClient,
        config: &WorkerConfig,
        handler: impl TaskHandler,
    ) -> Result<Self, ConsumerError> {
        let channel = client
            .connection()
            .create_channel()
            .await
            .map_err(|source| ConsumerError::OpenChannel {
                source: Box::new(source),
            })?;

        channel
            .basic_qos(config.prefetch, BasicQosOptions::default())
            .await
            .map_err(|source| ConsumerError::Prefetch {
                prefetch: config.prefetch,
                source: Box::new(source),
            })?;

        declare_worker_topology(&channel, config).await?;

        let broker_consumer = channel
            .basic_consume(
                config.queue.clone().into(),
                // Server-generated tag, like the TS wrapper's consume.
                "".into(),
                BasicConsumeOptions {
                    no_ack: false,
                    ..Default::default()
                },
                FieldTable::default(),
            )
            .await
            .map_err(|source| ConsumerError::Consume {
                queue: config.queue.clone(),
                source: Box::new(source),
            })?;

        tracing::info!(
            queue = %config.queue,
            exchange = %config.exchange,
            binding_key = %config.binding_key,
            prefetch = config.prefetch,
            consumer_tag = %broker_consumer.tag(),
            "Worker is running and waiting for messages"
        );

        let (trigger, watch) = shutdown::channel();
        let loop_task = tokio::spawn(consume_loop(
            client.publisher(),
            channel,
            broker_consumer,
            Arc::new(config.clone()),
            Arc::new(handler),
            watch,
        ));

        Ok(Self {
            shutdown: Some(trigger),
            loop_task: Some(loop_task),
        })
    }

    /// Signals cancellation and waits for the consume loop to finish: cancel
    /// the subscription, let in-flight handlers finish, then close the
    /// consumer channel. The connection belongs to the caller and is closed
    /// afterwards (see the module recipe).
    ///
    /// Safe to call more than once; a second call returns immediately.
    pub async fn shutdown(&mut self) -> Result<(), ConsumerError> {
        if let Some(trigger) = self.shutdown.take() {
            trigger.trigger();
        }
        self.wait_for_stop().await
    }

    /// Returns when the consume loop stops.
    ///
    /// Stops happen on shutdown (see [`Consumer::shutdown`]) or when the
    /// broker cancels the subscription; the result reports a stream that
    /// ended with an error. Dropping this future does not detach the loop.
    pub async fn wait_for_stop(&mut self) -> Result<(), ConsumerError> {
        let Some(task) = self.loop_task.as_mut() else {
            return Ok(());
        };

        // tokio's `JoinHandle` is only a `Future` by value; awaiting through
        // this borrowed wrapper keeps the handle owned by `self`, so dropping
        // the future (e.g. in `tokio::select!`) cannot detach the loop.
        let result =
            futures_lite::future::poll_fn(|context| Pin::new(&mut *task).poll(context)).await;
        self.loop_task.take();

        match result {
            Ok(result) => result,
            Err(source) => Err(ConsumerError::Task { source }),
        }
    }
}

impl Drop for Consumer {
    fn drop(&mut self) {
        // Without an explicit shutdown, dropping the trigger still wakes the
        // loop (its watch returns when the trigger is gone), which then
        // cancels, drains and closes on its own.
        self.shutdown.take();
    }
}

/// Declares the consumer's exchange, the `deadLetter` exchange and the
/// configured queue and binding, with the frozen arguments (ADR-0006).
/// Idempotent: redeclaring matching topology is a no-op on the broker.
async fn declare_worker_topology(
    channel: &Channel,
    config: &WorkerConfig,
) -> Result<(), ConsumerError> {
    for exchange in [config.exchange.as_str(), DEAD_LETTER_EXCHANGE] {
        channel
            .exchange_declare(
                exchange.into(),
                ExchangeKind::Topic,
                ExchangeDeclareOptions {
                    durable: true,
                    ..Default::default()
                },
                exchange_arguments(exchange),
            )
            .await
            .map_err(|source| ConsumerError::Declare {
                kind: "exchange",
                name: exchange.to_owned(),
                source: Box::new(source),
            })?;
    }

    channel
        .queue_declare(
            config.queue.clone().into(),
            QueueDeclareOptions {
                durable: true,
                ..Default::default()
            },
            queue_arguments(),
        )
        .await
        .map_err(|source| ConsumerError::Declare {
            kind: "queue",
            name: config.queue.clone(),
            source: Box::new(source),
        })?;

    channel
        .queue_bind(
            config.queue.clone().into(),
            config.exchange.clone().into(),
            config.binding_key.clone().into(),
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await
        .map_err(|source| ConsumerError::Declare {
            kind: "binding",
            name: config.binding_key.clone(),
            source: Box::new(source),
        })?;

    Ok(())
}

/// Spawns one delivery onto the in-flight set, sharing the publisher, handler
/// and worker config with the per-delivery task.
fn spawn_delivery<H: TaskHandler>(
    in_flight: &mut JoinSet<()>,
    publisher: &Publisher,
    handler: &Arc<H>,
    config: &Arc<WorkerConfig>,
    delivery: Delivery,
) {
    in_flight.spawn(handle_delivery(
        publisher.clone(),
        Arc::clone(handler),
        Arc::clone(config),
        delivery,
    ));
}

/// The consume loop: receive, spawn a handler task per delivery (in-flight
/// work is bounded by the prefetch limit), and stop on the shutdown signal.
async fn consume_loop<H: TaskHandler>(
    publisher: Publisher,
    channel: Channel,
    mut broker_consumer: BrokerConsumer,
    config: Arc<WorkerConfig>,
    handler: Arc<H>,
    mut shutdown: ShutdownWatch,
) -> Result<(), ConsumerError> {
    let mut in_flight: JoinSet<()> = JoinSet::new();
    let mut stream_error: Option<lapin::Error> = None;
    let mut requested = false;

    loop {
        tokio::select! {
            biased;
            _ = shutdown.wait() => {
                requested = true;
                break;
            }
            delivery = broker_consumer.next() => match delivery {
                Some(Ok(delivery)) => {
                    spawn_delivery(&mut in_flight, &publisher, &handler, &config, delivery);
                }
                Some(Err(error)) => {
                    stream_error = Some(error);
                    break;
                }
                // The broker cancelled the subscription (or the channel closed).
                None => break,
            },
            Some(joined) = in_flight.join_next(), if !in_flight.is_empty() => {
                if let Err(join_error) = joined {
                    tracing::error!(error = %join_error, "worker task failed");
                }
            }
        }
    }

    if requested {
        // Cancel first, close later (research §5). After the cancel-ok the
        // stream ends; deliveries the broker handed over before it are
        // already ours, so they are processed here rather than requeued by the
        // channel close.
        if let Err(error) = channel
            .basic_cancel(broker_consumer.tag(), BasicCancelOptions::default())
            .await
        {
            tracing::warn!(error = %error, "failed to cancel the consumer");
        }
        while let Some(delivery) = broker_consumer.next().await {
            match delivery {
                Ok(delivery) => {
                    spawn_delivery(&mut in_flight, &publisher, &handler, &config, delivery);
                }
                Err(error) => {
                    tracing::warn!(error = %error, "consumer stream failed while draining");
                    break;
                }
            }
        }
    } else {
        tracing::warn!(
            queue = %config.queue,
            "the broker cancelled the consumer; the consume loop stopped"
        );
    }

    // Let in-flight handlers finish before the channel goes away.
    while let Some(joined) = in_flight.join_next().await {
        if let Err(join_error) = joined {
            tracing::error!(error = %join_error, "worker task failed");
        }
    }

    if let Err(error) = channel.close(200, "shutdown".into()).await {
        tracing::warn!(error = %error, "failed to close the consumer channel");
    }

    match stream_error {
        Some(source) => Err(ConsumerError::Stream {
            source: Box::new(source),
        }),
        None => Ok(()),
    }
}

/// Handles one delivery: decode, run the handler, ack. On a decode failure
/// the handler is skipped entirely and the failure path records
/// `originalContent` as `"UNPARSEABLE"`. On handler failure the frozen DLQ
/// envelope is republished first; the ack happens on every path — success and
/// failure alike (`always ack`, no requeue).
async fn handle_delivery<H: TaskHandler>(
    publisher: Publisher,
    handler: Arc<H>,
    config: Arc<WorkerConfig>,
    delivery: Delivery,
) {
    let (task, failure) = match std::str::from_utf8(&delivery.data) {
        Ok(task) => {
            let task = task.to_owned();
            // Fastify parity: the "received" log happens only once the body
            // decoded; an unparseable body goes straight to the failure path.
            tracing::info!(queue = %config.queue, msg = %task, "Worker received message");
            match handler.handle(&task).await {
                Ok(()) => {
                    tracing::info!(queue = %config.queue, msg = %task, "Worker processed message");
                    (task, None)
                }
                Err(error) => {
                    tracing::error!(
                        queue = %config.queue,
                        msg = %task,
                        error = %error.message(),
                        error_name = error.name(),
                        "Worker failed to process message"
                    );
                    (task, Some(error))
                }
            }
        }
        Err(decode_error) => {
            let error = TaskError::new("Utf8Error", decode_error.to_string());
            tracing::error!(
                queue = %config.queue,
                msg = UNPARSEABLE,
                error = %error.message(),
                error_name = error.name(),
                "Worker failed to process message"
            );
            (UNPARSEABLE.to_owned(), Some(error))
        }
    };

    if let Some(error) = &failure {
        publish_dead_letter(&publisher, &config, error, &task).await;
    }

    if let Err(error) = delivery.ack(BasicAckOptions::default()).await {
        tracing::warn!(queue = %config.queue, error = %error, "failed to ack the message");
    }
}

/// Republishes a handler failure to `deadLetter` with routing key
/// `<queue>.failed`, awaiting the broker confirm. A failed publish is logged
/// and swallowed (the Fastify convention: log and continue); the delivery is
/// acked either way.
async fn publish_dead_letter(
    publisher: &Publisher,
    config: &WorkerConfig,
    error: &TaskError,
    original_content: &str,
) {
    let payload = DeadLetterPayload::for_failure(error, original_content, &config.queue);
    let body = match serde_json::to_vec(&payload) {
        Ok(body) => body,
        Err(serialize_error) => {
            tracing::error!(
                queue = %config.queue,
                error = %serialize_error,
                "worker failed to serialize the DLQ payload"
            );
            return;
        }
    };
    let routing_key = dead_letter_routing_key(&config.queue);
    if let Err(publish_error) = publisher
        .publish(DEAD_LETTER_EXCHANGE, &routing_key, &body)
        .await
    {
        tracing::error!(
            queue = %config.queue,
            error = %publish_error,
            "worker failed to publish to DLQ"
        );
    }
}

/// The routing key a failure of `queue` is republished with: `<queue>.failed`
/// (frozen).
fn dead_letter_routing_key(queue: &str) -> String {
    format!("{queue}.failed")
}

/// Example task handler, mirroring the TS template's `processTask`.
///
/// **Example — remove when building your service.**
pub struct DemoTaskHandler;

/// Result of [`DemoTaskHandler::process`], mirroring the TS demo's
/// `{ originalLength, processed }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DemoTaskResult {
    /// The decoded task's length (`originalLength` in the TS demo).
    pub original_length: usize,
    /// Whether the task was processed (always true in the TS demo).
    pub processed: bool,
}

impl DemoTaskHandler {
    /// Processes one task body, mirroring the TS demo's `processTask`.
    pub fn process(task: &str) -> DemoTaskResult {
        DemoTaskResult {
            original_length: task.len(),
            processed: true,
        }
    }
}

impl TaskHandler for DemoTaskHandler {
    async fn handle(&self, task: &str) -> Result<(), TaskError> {
        let result = Self::process(task);
        tracing::info!(
            original_length = result.original_length,
            processed = result.processed,
            "Demo task processed"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dead_letter_payload_uses_the_frozen_field_names() {
        let error = TaskError::new("BoomError", "boom");
        let payload = DeadLetterPayload::for_failure(&error, "original", "task_queue");

        let json = serde_json::to_value(&payload).expect("payload serializes");
        let object = json.as_object().expect("payload is an object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "error",
                "errorName",
                "failedAt",
                "originalContent",
                "queueName"
            ],
            "{json}"
        );
        assert_eq!(object["error"], "boom", "`error` is TaskError::message");
        assert_eq!(
            object["errorName"], "BoomError",
            "`errorName` is TaskError::name"
        );
        assert_eq!(object["originalContent"], "original");
        assert_eq!(object["queueName"], "task_queue");

        let failed_at = object["failedAt"].as_str().expect("failedAt is a string");
        assert_eq!(
            failed_at.len(),
            24,
            "failedAt with millis is 24 chars: {failed_at}"
        );
        chrono::DateTime::parse_from_rfc3339(failed_at)
            .unwrap_or_else(|error| panic!("failedAt {failed_at:?} is not ISO-8601: {error}"));
    }

    #[test]
    fn failed_at_matches_javascript_to_iso_string_exactly() {
        // 2023-01-02T03:04:05Z.
        const SECONDS: i64 = 1_672_628_645;

        let millis = DateTime::from_timestamp(SECONDS, 6_000_000).expect("timestamp");
        assert_eq!(format_iso8601(millis), "2023-01-02T03:04:05.006Z");

        // Zero millis still prints three digits, like `toISOString()`.
        let zero = DateTime::from_timestamp(SECONDS, 0).expect("timestamp");
        assert_eq!(format_iso8601(zero), "2023-01-02T03:04:05.000Z");

        // Sub-millisecond precision is truncated, never rounded.
        let nanos = DateTime::from_timestamp(SECONDS, 6_789_001).expect("timestamp");
        assert_eq!(format_iso8601(nanos), "2023-01-02T03:04:05.006Z");
    }

    #[test]
    fn dead_letter_routing_key_is_the_queue_name_dot_failed() {
        assert_eq!(dead_letter_routing_key("task_queue"), "task_queue.failed");
    }

    #[test]
    fn demo_task_handler_mirrors_the_typescript_demo() {
        let result = DemoTaskHandler::process("hello");
        assert_eq!(
            result,
            DemoTaskResult {
                original_length: 5,
                processed: true
            }
        );
    }
}
