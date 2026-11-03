//! AMQP publisher + topology against a live broker (ADR-0006).
//!
//! The broker-touching tests require the shared compose `infra` RabbitMQ on
//! `localhost:5672` (guest/guest). The broker-restart scenario is `#[ignore]`d
//! because it must be orchestrated from outside the container:
//!
//! ```sh
//! cargo test --test amqp survives_broker_restart -- --ignored --nocapture
//! # wait for `PHASE1_DONE`, then, from the host:
//! docker restart integration-rabbitmq-1
//! touch target/amqp-restart-trigger
//! ```

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use lapin::ExchangeKind;
use lapin::options::{
    BasicAckOptions, BasicGetOptions, ExchangeDeclareOptions, ExchangeDeleteOptions,
    QueueBindOptions, QueueDeclareOptions,
};
use lapin::types::{AMQPValue, FieldTable, LongString};

use home_users::amqp::AmqpClient;

use common::{broker, unique, verify_connection};

/// Serializes tests that mutate or observe the shared broker topology.
static BROKER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn table(entries: &[(&str, AMQPValue)]) -> FieldTable {
    let mut table = FieldTable::default();
    for (key, value) in entries {
        table.insert((*key).into(), value.clone());
    }
    table
}

/// amqplib's codec sends field-table strings as `longstr` (`S`).
fn long_string(value: &str) -> AMQPValue {
    AMQPValue::LongString(LongString::from(value))
}

/// Pulls messages from `task_queue` until `payload` shows up, acking whatever
/// passes through (including leftovers from earlier runs).
async fn receive_matching(channel: &lapin::Channel, payload: &str) -> (String, String) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match channel
            .basic_get("task_queue".into(), BasicGetOptions::default())
            .await
            .expect("basic_get")
        {
            Some(message) => {
                let matches = message.data == payload.as_bytes();
                let exchange = message.exchange.as_str().to_owned();
                let routing_key = message.routing_key.as_str().to_owned();
                message.ack(BasicAckOptions::default()).await.expect("ack");
                if matches {
                    return (exchange, routing_key);
                }
            }
            None => {
                assert!(
                    Instant::now() < deadline,
                    "message {payload:?} never arrived on task_queue"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

/// Redeclares the frozen topology with hardcoded ADR-0006 literals; RabbitMQ
/// answers 406 PRECONDITION_FAILED when any option or argument differs, so a
/// success proves the broker-side declaration matches exactly.
#[tokio::test]
async fn topology_matches_adr_0006_exactly() {
    let _guard = BROKER.lock().await;
    let client = AmqpClient::connect(&broker()).await.expect("connect");
    client.declare_topology().await.expect("declare topology");

    let (_connection, channel) = verify_connection().await;

    channel
        .exchange_declare(
            "main".into(),
            ExchangeKind::Topic,
            ExchangeDeclareOptions {
                durable: true,
                ..Default::default()
            },
            table(&[
                ("alternate-exchange", long_string("nonRouted")),
                ("x-message-ttl", AMQPValue::LongInt(86_400_000)),
            ]),
        )
        .await
        .expect("`main` must be a durable topic exchange with alternate-exchange `nonRouted` and a 24h TTL");

    channel
        .exchange_declare(
            "deadLetter".into(),
            ExchangeKind::Topic,
            ExchangeDeclareOptions {
                durable: true,
                ..Default::default()
            },
            FieldTable::default(),
        )
        .await
        .expect("`deadLetter` must be a durable topic exchange without arguments");

    let queue = channel
        .queue_declare(
            "task_queue".into(),
            QueueDeclareOptions {
                durable: true,
                ..Default::default()
            },
            table(&[
                ("x-message-ttl", AMQPValue::LongInt(3_600_000)),
                ("x-dead-letter-exchange", long_string("deadLetter")),
            ]),
        )
        .await
        .expect("`task_queue` must be durable with a 1h TTL and the `deadLetter` DLX");
    assert_eq!(queue.name().as_str(), "task_queue");

    channel
        .queue_bind(
            "task_queue".into(),
            "main".into(),
            "task.#".into(),
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await
        .expect("`task_queue` must be bound to `main` with `task.#`");
}

#[tokio::test]
async fn confirmed_publish_routes_to_task_queue() {
    let _guard = BROKER.lock().await;
    let client = AmqpClient::connect(&broker()).await.expect("connect");
    client.declare_topology().await.expect("declare topology");

    let payload = unique("t8-amqp-route");
    let confirmation = client
        .publish("main", "task.1", payload.as_bytes())
        .await
        .expect("confirmed publish");
    assert!(
        confirmation.is_ack(),
        "broker must ack the publish, got {confirmation:?}"
    );

    let (_connection, channel) = verify_connection().await;
    let (exchange, routing_key) = receive_matching(&channel, &payload).await;
    assert_eq!(exchange, "main");
    assert_eq!(routing_key, "task.1");
}

#[tokio::test]
async fn publish_declares_the_exchange_on_the_publish_path() {
    let _guard = BROKER.lock().await;
    let client = AmqpClient::connect(&broker()).await.expect("connect");
    client.declare_topology().await.expect("declare topology");

    // Delete the exchange out from under the publisher: only a publish path
    // that re-declares it (the TS wrapper's `declareExchange` parity) can
    // succeed.
    let (_connection, channel) = verify_connection().await;
    channel
        .exchange_delete("main".into(), ExchangeDeleteOptions::default())
        .await
        .expect("delete `main`");

    let result = client
        .publish("main", "task.parity", unique("t8-amqp-parity").as_bytes())
        .await;

    // Restore the frozen topology for the other tests before asserting.
    client
        .declare_topology()
        .await
        .expect("restore the topology");

    let confirmation = result.expect("publish must declare `main` first and succeed");
    assert!(
        confirmation.is_ack(),
        "broker must ack the publish, got {confirmation:?}"
    );
}

/// Orchestrated from outside: this test cannot restart the broker itself
/// (a cargo test runs inside the rust-dev container, without docker access).
#[ignore = "restarts the shared broker; orchestrate with docker restart + target/amqp-restart-trigger"]
#[tokio::test]
async fn survives_broker_restart() {
    let _guard = BROKER.lock().await;
    let client = AmqpClient::connect(&broker()).await.expect("connect");
    client.declare_topology().await.expect("declare topology");

    let phase1 = unique("t8-amqp-pre-restart");
    let start = Instant::now();
    client
        .publish("main", "task.restart", phase1.as_bytes())
        .await
        .expect("publish before restart");
    eprintln!(
        "PHASE1_DONE after {:?}: restart the broker, then `touch target/amqp-restart-trigger`",
        start.elapsed()
    );

    let trigger = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/target/amqp-restart-trigger"
    ));
    let deadline = Instant::now() + Duration::from_secs(300);
    while !trigger.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            trigger.display()
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    std::fs::remove_file(&trigger).ok();
    let triggered = Instant::now();

    let mut attempts = 0_u32;
    let confirmation = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            match client
                .publish("main", "task.restart", b"post-restart")
                .await
            {
                Ok(confirmation) if confirmation.is_ack() => return confirmation,
                other => {
                    attempts += 1;
                    if attempts <= 3 {
                        eprintln!("publish after restart not confirmed yet: {other:?}");
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .expect("the publisher must recover and confirm after the broker restarts");
    eprintln!(
        "RECOVERED after {:?} ({attempts} failed attempts, {confirmation:?})",
        triggered.elapsed()
    );

    // The recovered channel routes to the durable queue. (The payloads are
    // transient — the TS publisher never set a delivery mode — so the
    // pre-restart message itself is not expected to survive the restart.)
    let (_connection, channel) = verify_connection().await;
    let (exchange, routing_key) = receive_matching(&channel, "post-restart").await;
    assert_eq!(exchange, "main");
    assert_eq!(routing_key, "task.restart");
    eprintln!("ROUTED after {:?}", triggered.elapsed());
}

/// The service app under test: spawned from the real binary, logs to a file.
struct AppProcess {
    child: std::process::Child,
    log_path: PathBuf,
}

impl AppProcess {
    fn spawn(extra_env: &[(&str, &str)]) -> Self {
        let port = free_port();
        let log_path =
            std::env::temp_dir().join(format!("t8-amqp-app-{}-{port}.log", std::process::id()));
        let log = std::fs::File::create(&log_path).expect("create the log file");

        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_home-users"));
        command
            .env("API_PORT", port.to_string())
            .env("APP_LOG_LEVEL", "info")
            .stdout(std::process::Stdio::from(
                log.try_clone().expect("clone the log file"),
            ))
            .stderr(std::process::Stdio::from(log));
        for (key, value) in extra_env {
            command.env(key, value);
        }

        let child = command.spawn().expect("spawn the service app");
        Self { child, log_path }
    }

    fn logs(&self) -> String {
        std::fs::read_to_string(&self.log_path).unwrap_or_default()
    }

    async fn wait_for_log(&mut self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if self.logs().contains(needle) {
                return;
            }
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                panic!(
                    "app exited with {status} before logging {needle:?}; logs:\n{}",
                    self.logs()
                );
            }
            assert!(
                Instant::now() < deadline,
                "app never logged {needle:?}; logs:\n{}",
                self.logs()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn sigterm_and_wait(&mut self) -> (std::process::ExitStatus, Duration) {
        let pid = self.child.id();
        let started = Instant::now();
        let killed = std::process::Command::new("sh")
            .args(["-c", &format!("kill -TERM {pid}")])
            .status()
            .expect("send SIGTERM");
        assert!(killed.success(), "kill -TERM {pid} failed");

        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return (status, started.elapsed());
            }
            assert!(
                started.elapsed() < Duration::from_secs(15),
                "app must exit after SIGTERM (no hang); logs:\n{}",
                self.logs()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

impl Drop for AppProcess {
    fn drop(&mut self) {
        // kill_on_drop is tokio-only; the std child needs an explicit kill.
        // Reaping here matters on the panic path, though the shared
        // container's PID 1 (`sleep`) never reaps orphans.
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_file(&self.log_path);
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

#[tokio::test]
async fn sigterm_closes_the_amqp_connection_and_exits_cleanly() {
    let _guard = BROKER.lock().await;
    // Same convention as `broker()`: the shared host-side stack by default,
    // `RABBIT_HOST`/`RABBIT_PORT` when set (e.g. the compose acceptance run).
    let host = std::env::var("RABBIT_HOST").unwrap_or_else(|_| "localhost".to_owned());
    let port = std::env::var("RABBIT_PORT").unwrap_or_else(|_| "5672".to_owned());
    let mut app = AppProcess::spawn(&[
        ("RABBIT_DISABLED", "false"),
        ("RABBIT_HOST", &host),
        ("RABBIT_PORT", &port),
        ("RABBIT_USER", "guest"),
        ("RABBIT_PASSWORD", "guest"),
        ("RABBIT_TRANSPORT", "amqp"),
    ]);

    // `RabbitMQ connected` is logged after the shutdown watcher is installed,
    // so a SIGTERM from this point on is always handled gracefully.
    app.wait_for_log("RabbitMQ connected").await;
    app.wait_for_log("RabbitMQ topology declared").await;

    let (status, elapsed) = app.sigterm_and_wait().await;
    assert!(
        status.success(),
        "app exited with {status}; logs:\n{}",
        app.logs()
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "shutdown took {elapsed:?}"
    );

    let logs = app.logs();
    assert!(logs.contains("RabbitMQ connection closed"), "logs:\n{logs}");
    assert!(!logs.contains("cleanup task timed out"), "logs:\n{logs}");
}

#[tokio::test]
async fn disabled_gate_skips_connecting_entirely() {
    let mut app = AppProcess::spawn(&[
        ("RABBIT_DISABLED", "true"),
        // Unreachable on purpose: any connection attempt would fail startup.
        ("RABBIT_HOST", "127.0.0.1"),
        ("RABBIT_PORT", "1"),
    ]);

    app.wait_for_log("listening").await;
    // Let the signal watcher install; nothing else is pending after `listening`.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let (status, _) = app.sigterm_and_wait().await;
    assert!(
        status.success(),
        "app exited with {status}; logs:\n{}",
        app.logs()
    );
    assert!(
        !app.logs().contains("RabbitMQ connected"),
        "a disabled RabbitMQ must not be connected to; logs:\n{}",
        app.logs()
    );
}
