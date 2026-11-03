//! Consumer + DLQ semantics against a live broker (ADR-0006).
//!
//! The broker-touching tests require the shared compose `infra` RabbitMQ on
//! `localhost:5672` (guest/guest), like `tests/amqp.rs`.
//!
//! Every test runs on its own freshly declared queue name, so these tests
//! never race the publisher tests in `tests/amqp.rs` for messages on
//! `task_queue`.

mod common;

use std::time::{Duration, Instant};

use lapin::Queue;
use lapin::options::{BasicGetOptions, QueueDeclareOptions, QueueDeleteOptions};
use lapin::types::FieldTable;

use home_orders::amqp::connect_and_declare;
use home_orders::config::WorkerConfig;
use home_orders::consumer::{Consumer, TaskError, TaskHandler};

use common::{broker, declare_dlq_probe, unique, verify_connection};

/// Serializes tests that declare queues or observe the shared broker.
static BROKER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The worker config these tests drive: a unique queue bound to `main` under
/// a unique routing key, so nothing else on the shared broker interferes.
fn worker_config(queue: &str) -> WorkerConfig {
    WorkerConfig {
        queue: queue.to_owned(),
        exchange: "main".to_owned(),
        binding_key: format!("t9.{queue}"),
        prefetch: 1,
    }
}

async fn receive_from(channel: &lapin::Channel, queue: &str) -> Option<Vec<u8>> {
    match channel
        .basic_get(queue.to_owned().into(), BasicGetOptions::default())
        .await
        .expect("basic_get")
    {
        Some(message) => {
            let lapin::message::BasicGetMessage { delivery, .. } = message;
            // Leave the broker-side state clean for the next assertion.
            delivery
                .ack(lapin::options::BasicAckOptions::default())
                .await
                .expect("ack the probe message");
            Some(delivery.data)
        }
        None => None,
    }
}

/// The worker queue's broker-side state, via a passive declare.
async fn queue_state(channel: &lapin::Channel, queue: &str) -> Queue {
    channel
        .queue_declare(
            queue.to_owned().into(),
            QueueDeclareOptions {
                passive: true,
                ..Default::default()
            },
            FieldTable::default(),
        )
        .await
        .unwrap_or_else(|error| panic!("passive declare of {queue:?} failed: {error}"))
}

/// Waits until the broker reports `consumers` active consumers on `queue`,
/// returning the last observed state.
async fn wait_for_consumer_count(channel: &lapin::Channel, queue: &str, consumers: u32) -> Queue {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = queue_state(channel, queue).await;
        if state.consumer_count() == consumers {
            return state;
        }
        assert!(
            Instant::now() < deadline,
            "queue {queue:?} never reached {consumers} consumers (last: {} consumers, {} messages)",
            state.consumer_count(),
            state.message_count()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Records every task body it sees; optionally fails for specific bodies and
/// optionally blocks until a gate opens.
struct RecordingHandler {
    seen: tokio::sync::mpsc::UnboundedSender<String>,
    fail_on: Vec<String>,
    gate: Option<tokio::sync::watch::Receiver<bool>>,
}

impl RecordingHandler {
    fn new() -> (Self, tokio::sync::mpsc::UnboundedReceiver<String>) {
        let (seen, receiver) = tokio::sync::mpsc::unbounded_channel();
        let handler = Self {
            seen,
            fail_on: Vec::new(),
            gate: None,
        };
        (handler, receiver)
    }

    fn failing_on(mut self, task: &str) -> Self {
        self.fail_on.push(task.to_owned());
        self
    }

    fn gated(mut self, gate: tokio::sync::watch::Receiver<bool>) -> Self {
        self.gate = Some(gate);
        self
    }
}

impl TaskHandler for RecordingHandler {
    async fn handle(&self, task: &str) -> Result<(), TaskError> {
        let _ = self.seen.send(task.to_owned());
        if let Some(gate) = &self.gate {
            let mut gate = gate.clone();
            let _ = gate.wait_for(|open| *open).await;
        }
        if self.fail_on.iter().any(|fail| fail == task) {
            return Err(TaskError::new("BoomError", "boom"));
        }
        Ok(())
    }
}

/// Polls `queue` until a message arrives, then acks it and returns its body.
async fn wait_for_probe_message(channel: &lapin::Channel, queue: &str) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(body) = receive_from(channel, queue).await {
            return body;
        }
        assert!(
            Instant::now() < deadline,
            "no message arrived on {queue:?} within the deadline"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn shutdown_waits_for_in_flight_handlers_and_leaves_no_orphan_consumer() {
    let _guard = BROKER.lock().await;
    let queue = unique("t9-shutdown");
    let config = worker_config(&queue);

    let client = connect_and_declare(&broker())
        .await
        .expect("consumer client");
    let publisher = connect_and_declare(&broker())
        .await
        .expect("publisher client");
    let (_connection, verify) = verify_connection().await;

    // The handler blocks until `release` opens, so "in flight" is provable.
    let (release, release_rx) = tokio::sync::watch::channel(false);
    let (handler, mut seen) = RecordingHandler::new();
    let mut worker = Consumer::start(&client, &config, handler.gated(release_rx))
        .await
        .expect("start the consumer");
    wait_for_consumer_count(&verify, &queue, 1).await;

    publisher
        .publish("main", &config.binding_key, b"blocked")
        .await
        .expect("publish");
    tokio::time::timeout(Duration::from_secs(10), seen.recv())
        .await
        .expect("the handler must run")
        .expect("recorded task");

    // The service-app seam: the consumer's shutdown is a cleanup hook, run by
    // the same registry the AMQP client's cleanup uses (in registration
    // order, consumer before client).
    let mut cleanup = home_orders::shutdown::CleanupRegistry::new();
    cleanup.register("amqp-consumer", async move {
        worker.shutdown().await.expect("clean consumer shutdown");
    });
    let cleanup = tokio::spawn(cleanup.run_all(home_orders::shutdown::CLEANUP_TIMEOUT));

    // The cleanup must not finish while the handler is still running.
    let blocked = Instant::now();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !cleanup.is_finished(),
        "shutdown must let the in-flight handler finish, not abandon it"
    );
    eprintln!(
        "EVIDENCE shutdown: cleanup still waiting for the in-flight handler after {:?}",
        blocked.elapsed()
    );

    release.send(true).expect("release the handler");
    let released = Instant::now();
    tokio::time::timeout(Duration::from_secs(10), cleanup)
        .await
        .expect("shutdown must not hang")
        .expect("cleanup task must finish");
    eprintln!(
        "EVIDENCE shutdown: cleanup finished {:?} after the handler was released",
        released.elapsed()
    );

    // No orphan consumer, and the in-flight delivery was acked before the
    // channel closed.
    let state = wait_for_consumer_count(&verify, &queue, 0).await;
    assert_eq!(
        state.message_count(),
        0,
        "the in-flight delivery must be acked before the channel closes"
    );

    verify
        .queue_delete(queue.into(), QueueDeleteOptions::default())
        .await
        .expect("delete the test queue");
}

#[tokio::test]
async fn success_path_acks_without_a_dlq_entry() {
    let _guard = BROKER.lock().await;
    let queue = unique("t9-success");

    let client = connect_and_declare(&broker())
        .await
        .expect("consumer client");
    let publisher = connect_and_declare(&broker())
        .await
        .expect("publisher client");
    let (_connection, verify) = verify_connection().await;
    let dlq_probe = declare_dlq_probe(&verify, &format!("{queue}.failed")).await;

    let (handler, mut seen) = RecordingHandler::new();
    let mut worker = Consumer::start(&client, &worker_config(&queue), handler)
        .await
        .expect("start the consumer");
    wait_for_consumer_count(&verify, &queue, 1).await;

    let payload = "hello task";
    publisher
        .publish(
            "main",
            &worker_config(&queue).binding_key,
            payload.as_bytes(),
        )
        .await
        .expect("publish");

    let handled = tokio::time::timeout(Duration::from_secs(10), seen.recv())
        .await
        .expect("the handler must run")
        .expect("recorded task");
    assert_eq!(handled, payload);

    // The failure path is the only writer to `deadLetter`: a success must not
    // leave a DLQ entry behind.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        receive_from(&verify, &dlq_probe).await.is_none(),
        "a successful message must not be republished to the DLQ"
    );

    worker.shutdown().await.expect("clean shutdown");

    // Ack proof: closing the consumer's channel requeues unacked messages, so
    // an empty queue after shutdown means the delivery was really acked.
    let state = wait_for_consumer_count(&verify, &queue, 0).await;
    assert_eq!(
        state.message_count(),
        0,
        "an acked message must not be requeued on shutdown"
    );

    verify
        .queue_delete(queue.into(), QueueDeleteOptions::default())
        .await
        .expect("delete the test queue");
}

#[tokio::test]
async fn handler_failure_republishes_the_frozen_dlq_payload_then_acks() {
    let _guard = BROKER.lock().await;
    let queue = unique("t9-failure");
    let config = worker_config(&queue);

    let client = connect_and_declare(&broker())
        .await
        .expect("consumer client");
    let publisher = connect_and_declare(&broker())
        .await
        .expect("publisher client");
    let (_connection, verify) = verify_connection().await;
    let dlq_probe = declare_dlq_probe(&verify, &format!("{queue}.failed")).await;

    let (handler, mut seen) = RecordingHandler::new();
    let mut worker = Consumer::start(&client, &config, handler.failing_on("fail-me"))
        .await
        .expect("start the consumer");
    wait_for_consumer_count(&verify, &queue, 1).await;

    publisher
        .publish("main", &config.binding_key, b"fail-me")
        .await
        .expect("publish");
    tokio::time::timeout(Duration::from_secs(10), seen.recv())
        .await
        .expect("the handler must run")
        .expect("recorded task");

    // The frozen envelope on `<queue>.failed`.
    let body = wait_for_probe_message(&verify, &dlq_probe).await;
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("DLQ payload is JSON");
    eprintln!("EVIDENCE dlq payload on {queue}.failed: {payload}");
    let object = payload.as_object().expect("DLQ payload is an object");
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
        "the payload carries exactly the frozen fields, got {payload}"
    );
    assert_eq!(object["error"], "boom");
    assert_eq!(object["errorName"], "BoomError");
    assert_eq!(object["originalContent"], "fail-me");
    assert_eq!(object["queueName"], queue);
    let failed_at = object["failedAt"].as_str().expect("failedAt is a string");
    chrono::DateTime::parse_from_rfc3339(failed_at)
        .unwrap_or_else(|error| panic!("failedAt {failed_at:?} is not ISO-8601: {error}"));

    // Always ack, no requeue: the failed delivery must not come back.
    worker.shutdown().await.expect("clean shutdown");
    let state = wait_for_consumer_count(&verify, &queue, 0).await;
    assert_eq!(
        state.message_count(),
        0,
        "the failed delivery must be acked, never requeued"
    );

    verify
        .queue_delete(queue.into(), QueueDeleteOptions::default())
        .await
        .expect("delete the test queue");
}

#[tokio::test]
async fn prefetch_one_holds_the_queue_while_the_handler_blocks() {
    let _guard = BROKER.lock().await;
    let queue = unique("t9-prefetch-1");
    let config = WorkerConfig {
        prefetch: 1,
        ..worker_config(&queue)
    };

    let client = connect_and_declare(&broker())
        .await
        .expect("consumer client");
    let publisher = connect_and_declare(&broker())
        .await
        .expect("publisher client");
    let (_connection, verify) = verify_connection().await;

    let (gate, gate_rx) = tokio::sync::watch::channel(false);
    let (handler, mut seen) = RecordingHandler::new();
    let mut worker = Consumer::start(&client, &config, handler.gated(gate_rx))
        .await
        .expect("start the consumer");
    wait_for_consumer_count(&verify, &queue, 1).await;

    for index in 0..4 {
        publisher
            .publish(
                "main",
                &config.binding_key,
                format!("task-{index}").as_bytes(),
            )
            .await
            .expect("publish");
    }

    // The first handler invocation blocks on the gate; with prefetch 1 the
    // broker must not hand out a second message while the first is unacked.
    let first = tokio::time::timeout(Duration::from_secs(10), seen.recv())
        .await
        .expect("the handler must run")
        .expect("recorded task");
    assert_eq!(first, "task-0");
    tokio::time::sleep(Duration::from_millis(500)).await;
    eprintln!(
        "EVIDENCE prefetch=1: deliveries observed while the handler is blocked: 1 (queued back: 3)"
    );
    assert!(
        seen.try_recv().is_err(),
        "WORKER_PREFETCH=1 must hold the remaining messages back while one is unacked"
    );

    // Unblock: every message is processed and acked in order.
    gate.send(true).expect("open the gate");
    for index in 1..4 {
        let handled = tokio::time::timeout(Duration::from_secs(10), seen.recv())
            .await
            .expect("the remaining messages must be processed")
            .expect("recorded task");
        assert_eq!(handled, format!("task-{index}"));
    }

    worker.shutdown().await.expect("clean shutdown");
    let state = wait_for_consumer_count(&verify, &queue, 0).await;
    assert_eq!(state.message_count(), 0, "every message must be acked");

    verify
        .queue_delete(queue.into(), QueueDeleteOptions::default())
        .await
        .expect("delete the test queue");
}

#[tokio::test]
async fn prefetch_three_hands_out_three_while_the_handlers_block() {
    let _guard = BROKER.lock().await;
    let queue = unique("t9-prefetch-3");
    let config = WorkerConfig {
        prefetch: 3,
        ..worker_config(&queue)
    };

    let client = connect_and_declare(&broker())
        .await
        .expect("consumer client");
    let publisher = connect_and_declare(&broker())
        .await
        .expect("publisher client");
    let (_connection, verify) = verify_connection().await;

    let (gate, gate_rx) = tokio::sync::watch::channel(false);
    let (handler, mut seen) = RecordingHandler::new();
    let mut worker = Consumer::start(&client, &config, handler.gated(gate_rx))
        .await
        .expect("start the consumer");
    wait_for_consumer_count(&verify, &queue, 1).await;

    for index in 0..4 {
        publisher
            .publish(
                "main",
                &config.binding_key,
                format!("task-{index}").as_bytes(),
            )
            .await
            .expect("publish");
    }

    // Three handlers block on the gate; the fourth delivery is held back.
    for index in 0..3 {
        let handled = tokio::time::timeout(Duration::from_secs(10), seen.recv())
            .await
            .expect("the first three deliveries must run concurrently")
            .expect("recorded task");
        assert_eq!(handled, format!("task-{index}"));
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    eprintln!(
        "EVIDENCE prefetch=3: deliveries observed while the handlers are blocked: 3 (queued back: 1)"
    );
    assert!(
        seen.try_recv().is_err(),
        "WORKER_PREFETCH=3 must hold the fourth message back"
    );

    gate.send(true).expect("open the gate");
    let fourth = tokio::time::timeout(Duration::from_secs(10), seen.recv())
        .await
        .expect("the fourth message must be processed after the acks")
        .expect("recorded task");
    assert_eq!(fourth, "task-3");

    worker.shutdown().await.expect("clean shutdown");
    let state = wait_for_consumer_count(&verify, &queue, 0).await;
    assert_eq!(state.message_count(), 0, "every message must be acked");

    verify
        .queue_delete(queue.into(), QueueDeleteOptions::default())
        .await
        .expect("delete the test queue");
}

#[tokio::test]
async fn unparseable_payload_lands_in_the_dlq_and_the_consumer_keeps_running() {
    let _guard = BROKER.lock().await;
    let queue = unique("t9-unparseable");
    let config = worker_config(&queue);

    let client = connect_and_declare(&broker())
        .await
        .expect("consumer client");
    let publisher = connect_and_declare(&broker())
        .await
        .expect("publisher client");
    let (_connection, verify) = verify_connection().await;
    let dlq_probe = declare_dlq_probe(&verify, &format!("{queue}.failed")).await;

    let (handler, mut seen) = RecordingHandler::new();
    let mut worker = Consumer::start(&client, &config, handler)
        .await
        .expect("start the consumer");
    wait_for_consumer_count(&verify, &queue, 1).await;

    // Bytes that are not valid UTF-8.
    publisher
        .publish("main", &config.binding_key, &[0xff, 0xfe, 0x00, b'x'])
        .await
        .expect("publish");

    let body = wait_for_probe_message(&verify, &dlq_probe).await;
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("DLQ payload is JSON");
    assert_eq!(
        payload["originalContent"], "UNPARSEABLE",
        "a body that does not decode must be recorded as UNPARSEABLE, got {payload}"
    );
    assert_eq!(payload["queueName"], queue);
    assert_eq!(payload["errorName"], "Utf8Error");
    let error = payload["error"].as_str().expect("error is a string");
    assert!(
        error.contains("utf-8"),
        "the DLQ error must describe the decode failure, got {error:?}"
    );
    assert!(
        seen.try_recv().is_err(),
        "the handler must not run for an unparseable body"
    );

    // The consumer keeps running: the next, valid message is processed.
    publisher
        .publish("main", &config.binding_key, b"still alive")
        .await
        .expect("publish");
    let handled = tokio::time::timeout(Duration::from_secs(10), seen.recv())
        .await
        .expect("the handler must keep running after a failure")
        .expect("recorded task");
    assert_eq!(handled, "still alive");

    worker.shutdown().await.expect("clean shutdown");
    let state = wait_for_consumer_count(&verify, &queue, 0).await;
    assert_eq!(
        state.message_count(),
        0,
        "the unparseable delivery must be acked, never requeued"
    );

    verify
        .queue_delete(queue.into(), QueueDeleteOptions::default())
        .await
        .expect("delete the test queue");
}
