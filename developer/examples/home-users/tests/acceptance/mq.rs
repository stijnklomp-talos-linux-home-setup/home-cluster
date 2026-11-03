//! Publish → consume → DLQ against the real broker, through the real
//! [`AmqpClient`] publisher and [`Consumer`] loop (ADR-0006 semantics).
//!
//! The queue and binding are unique per run, so the test neither races other
//! runs nor depends on the default `task_queue` topology.

use std::time::{Duration, Instant};

use lapin::Confirmation;
use lapin::options::{BasicAckOptions, BasicGetOptions, QueueDeclareOptions, QueueDeleteOptions};
use lapin::types::FieldTable;

use home_users::amqp::{AmqpClient, connect_and_declare};
use home_users::config::WorkerConfig;
use home_users::consumer::{Consumer, TaskError, TaskHandler};

use crate::common::declare_dlq_probe;
use crate::{rabbit_config, unique_label};

const SUCCESS_BODY: &str = r#"{"acceptance":"ok"}"#;
const FAIL_BODY: &str = r#"{"acceptance":"fail"}"#;

/// Records every body it handles and fails on [`FAIL_BODY`], so one test can
/// drive both the success and the dead-letter path.
struct RecordingHandler {
    seen: tokio::sync::mpsc::UnboundedSender<String>,
}

impl TaskHandler for RecordingHandler {
    async fn handle(&self, task: &str) -> Result<(), TaskError> {
        let _ = self.seen.send(task.to_owned());
        if task == FAIL_BODY {
            return Err(TaskError::new("AcceptanceError", "acceptance failure"));
        }
        Ok(())
    }
}

/// A second connection used to verify broker state from outside the consumer.
async fn verification_channel(client: &AmqpClient) -> lapin::Channel {
    client
        .connection()
        .create_channel()
        .await
        .expect("open a verification channel")
}

/// Polls `queue` until a message arrives, acks it and returns its body.
async fn wait_for_dlq_message(channel: &lapin::Channel, queue: &str) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(message) = channel
            .basic_get(queue.into(), BasicGetOptions::default())
            .await
            .expect("basic_get on the DLQ probe")
        {
            message
                .delivery
                .ack(BasicAckOptions::default())
                .await
                .expect("ack the DLQ probe message");
            return message.delivery.data;
        }
        assert!(
            Instant::now() < deadline,
            "no message arrived on the DLQ probe {queue:?} within the deadline"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits until the broker reports `queue` with no ready messages left
/// (delivered-and-unacked messages are not required to be zero: an ack may
/// still be in flight when this returns, but a requeued message would show
/// up as ready).
async fn wait_for_empty_queue(channel: &lapin::Channel, queue: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = channel
            .queue_declare(
                queue.into(),
                QueueDeclareOptions {
                    passive: true,
                    ..Default::default()
                },
                FieldTable::default(),
            )
            .await
            .unwrap_or_else(|error| panic!("passive declare of {queue:?} failed: {error}"));
        if state.message_count() == 0 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "queue {queue:?} still holds {} messages",
            state.message_count()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn publish_consume_and_dead_letter_flow_against_the_real_broker() {
    let client = connect_and_declare(&rabbit_config())
        .await
        .expect("the acceptance broker must be reachable");

    let queue = unique_label("acceptance-queue");
    let binding_key = format!("acceptance.{queue}");
    let worker = WorkerConfig {
        queue: queue.clone(),
        exchange: "main".to_owned(),
        binding_key: binding_key.clone(),
        prefetch: 1,
    };

    let (seen_tx, mut seen_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut consumer = Consumer::start(&client, &worker, RecordingHandler { seen: seen_tx })
        .await
        .expect("start the consumer");

    // Publish → consume: the message reaches the handler and is acked.
    let confirmation = client
        .publish("main", &binding_key, SUCCESS_BODY.as_bytes())
        .await
        .expect("publish the success message");
    assert!(
        matches!(confirmation, Confirmation::Ack(_)),
        "the broker must confirm the publish"
    );

    let observed = tokio::time::timeout(Duration::from_secs(15), seen_rx.recv())
        .await
        .expect("the handler must see the success message within the deadline")
        .expect("the handler channel must stay open");
    assert_eq!(observed, SUCCESS_BODY);

    let channel = verification_channel(&client).await;
    wait_for_empty_queue(&channel, &queue).await;

    // Publish → consume → DLQ: the handler fails, the message is republished
    // to `deadLetter` with the frozen payload, and the original is acked.
    let probe = declare_dlq_probe(&channel, &format!("{queue}.failed")).await;

    client
        .publish("main", &binding_key, FAIL_BODY.as_bytes())
        .await
        .expect("publish the failing message");

    let observed = tokio::time::timeout(Duration::from_secs(15), seen_rx.recv())
        .await
        .expect("the handler must see the failing message within the deadline")
        .expect("the handler channel must stay open");
    assert_eq!(observed, FAIL_BODY);

    let payload = wait_for_dlq_message(&channel, &probe).await;
    let payload = serde_json::from_slice::<serde_json::Value>(&payload)
        .expect("the dead-letter payload must be JSON");
    assert_eq!(payload["error"], "acceptance failure");
    assert_eq!(payload["errorName"], "AcceptanceError");
    assert_eq!(payload["originalContent"], FAIL_BODY);
    assert_eq!(payload["queueName"], queue);
    let failed_at = payload["failedAt"]
        .as_str()
        .expect("failedAt must be a string");
    assert_eq!(failed_at.len(), 24, "failedAt with millis: {failed_at}");
    chrono::DateTime::parse_from_rfc3339(failed_at)
        .unwrap_or_else(|error| panic!("failedAt {failed_at:?} is not ISO-8601: {error}"));

    // The failed delivery was acked (never requeued): the queue drains.
    wait_for_empty_queue(&channel, &queue).await;

    consumer.shutdown().await.expect("shut the consumer down");
    channel
        .queue_delete(queue.clone().into(), QueueDeleteOptions::default())
        .await
        .expect("delete the acceptance queue");
    client.close().await.expect("close the AMQP connection");
}
