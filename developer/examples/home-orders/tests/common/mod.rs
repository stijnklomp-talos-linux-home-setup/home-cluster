//! Helpers shared by the real-broker integration tests (`tests/amqp.rs`,
//! `tests/consumer.rs`) and the acceptance suite (`tests/acceptance/mq.rs`).
//! Integration-test crates cannot share modules through the library, so each
//! `mod common;` compiles this file into its own test binary; every crate uses
//! a subset, hence the blanket `dead_code` allowance.

#![allow(dead_code)]

use std::time::{SystemTime, UNIX_EPOCH};

use lapin::options::{QueueBindOptions, QueueDeclareOptions};
use lapin::types::FieldTable;
use lapin::{Connection, ConnectionProperties};

use home_orders::config::{RabbitConfig, RabbitTransport};

/// Broker settings from the environment, defaulting to the shared compose
/// `infra` stack on `localhost:5672` (guest/guest); `RABBIT_HOST`/`RABBIT_PORT`
/// point at the in-network service in the compose acceptance run.
pub fn broker() -> RabbitConfig {
    RabbitConfig {
        enabled: true,
        host: std::env::var("RABBIT_HOST").unwrap_or_else(|_| "localhost".to_owned()),
        port: std::env::var("RABBIT_PORT")
            .ok()
            .and_then(|port| port.parse().ok())
            .unwrap_or(5672),
        user: "guest".to_owned(),
        password: "guest".to_owned(),
        transport: RabbitTransport::Amqp,
    }
}

/// A second, independent connection used to verify what a client did.
pub async fn verify_connection() -> (Connection, lapin::Channel) {
    let config = broker();
    let address = format!(
        "amqp://{}:{}@{}:{}",
        config.user, config.password, config.host, config.port
    );
    let connection = Connection::connect(&address, ConnectionProperties::default())
        .await
        .expect("verification connection");
    let channel = connection
        .create_channel()
        .await
        .expect("verification channel");
    (connection, channel)
}

/// A name no other test run uses (leftovers from failed runs are harmless:
/// they are never re-declared, only unique names are).
pub fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}-{}-{nanos}", std::process::id())
}

/// Declares an exclusive, auto-deleting probe queue bound to `deadLetter`
/// with `routing_key`, to observe DLQ writes.
pub async fn declare_dlq_probe(channel: &lapin::Channel, routing_key: &str) -> String {
    let probe = unique("dlq-probe");
    channel
        .queue_declare(
            probe.clone().into(),
            QueueDeclareOptions {
                exclusive: true,
                auto_delete: true,
                ..Default::default()
            },
            FieldTable::default(),
        )
        .await
        .expect("declare the DLQ probe queue");
    channel
        .queue_bind(
            probe.clone().into(),
            "deadLetter".into(),
            routing_key.to_owned().into(),
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await
        .expect("bind the DLQ probe queue");
    probe
}
