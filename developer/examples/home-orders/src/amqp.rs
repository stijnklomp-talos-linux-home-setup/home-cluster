//! AMQP publisher and the frozen topology (ADR-0006).
//!
//! [`AmqpClient`] owns one connection with automatic recovery
//! ([`lapin::ConnectionProperties::enable_auto_recover`]) and one publish
//! channel in confirm mode. Topology declarations port the Fastify wrapper
//! 1:1; consumer deployments (a documented `src/bin/worker.rs`, ticket #9)
//! reuse [`connect_and_declare`] and [`AmqpClient::connection`] to open their
//! own channel.

use lapin::options::{
    BasicPublishOptions, ConfirmSelectOptions, ExchangeDeclareOptions, QueueBindOptions,
    QueueDeclareOptions,
};
use lapin::types::{AMQPValue, FieldTable};
use lapin::{
    BasicProperties, Channel, Confirmation, Connection, ConnectionProperties, ExchangeKind,
};
use thiserror::Error;

use crate::config::RabbitConfig;

/// Exchange messages are published to (frozen name).
pub const MAIN_EXCHANGE: &str = "main";
/// Exchange failed messages are republished to (frozen name).
pub const DEAD_LETTER_EXCHANGE: &str = "deadLetter";
/// Alternate exchange for unroutable messages (frozen name). The Fastify
/// template only references it as an argument of [`MAIN_EXCHANGE`]; it does
/// not declare it either.
pub const NON_ROUTED_EXCHANGE: &str = "nonRouted";
/// Queue worker deployments consume from (frozen name).
pub const TASK_QUEUE: &str = "task_queue";
/// Binding key of [`TASK_QUEUE`] on [`MAIN_EXCHANGE`] (frozen).
pub const TASK_BINDING_KEY: &str = "task.#";

/// `main` exchange message TTL, 24 hours (frozen). `i32` mirrors amqplib's
/// wire type for integral numbers below 2^31.
const MAIN_EXCHANGE_TTL_MS: i32 = 86_400_000;
/// `task_queue` message TTL, 1 hour (frozen). `i32` mirrors amqplib.
const TASK_QUEUE_TTL_MS: i32 = 3_600_000;

fn main_exchange_arguments() -> FieldTable {
    let mut arguments = FieldTable::default();
    arguments.insert(
        "alternate-exchange".into(),
        AMQPValue::LongString(NON_ROUTED_EXCHANGE.into()),
    );
    arguments.insert(
        "x-message-ttl".into(),
        AMQPValue::LongInt(MAIN_EXCHANGE_TTL_MS),
    );
    arguments
}

/// Frozen queue arguments (`x-message-ttl`, `x-dead-letter-exchange`): every
/// queue a deployment declares through the wrapper carries them. Ticket #9's
/// consumer reuses this for `WORKER_QUEUE`.
pub(crate) fn queue_arguments() -> FieldTable {
    let mut arguments = FieldTable::default();
    arguments.insert(
        "x-message-ttl".into(),
        AMQPValue::LongInt(TASK_QUEUE_TTL_MS),
    );
    arguments.insert(
        "x-dead-letter-exchange".into(),
        AMQPValue::LongString(DEAD_LETTER_EXCHANGE.into()),
    );
    arguments
}

/// Argument set an exchange is declared with: `main` carries the frozen
/// alternate-exchange and 24h-TTL arguments; every other exchange (including
/// `deadLetter`) is declared without arguments. Ticket #9's consumer reuses
/// this for `WORKER_EXCHANGE`.
pub(crate) fn exchange_arguments(exchange: &str) -> FieldTable {
    match exchange {
        MAIN_EXCHANGE => main_exchange_arguments(),
        _ => FieldTable::default(),
    }
}

/// An AMQP operation failed.
#[derive(Debug, Error)]
pub enum AmqpError {
    /// Establishing the connection failed.
    #[error("failed to connect to the AMQP broker at {host}:{port}: {source}")]
    Connect {
        /// Broker host dialed.
        host: String,
        /// Broker port dialed.
        port: u16,
        /// Underlying lapin error (boxed: it dwarfs every other field).
        #[source]
        source: Box<lapin::Error>,
    },
    /// Opening the publish channel failed.
    #[error("failed to open the AMQP publish channel: {source}")]
    OpenChannel {
        /// Underlying lapin error (boxed: it dwarfs every other field).
        #[source]
        source: Box<lapin::Error>,
    },
    /// Enabling publisher confirms failed.
    #[error("failed to enable publisher confirms: {source}")]
    EnableConfirms {
        /// Underlying lapin error (boxed: it dwarfs every other field).
        #[source]
        source: Box<lapin::Error>,
    },
    /// Declaring an exchange, queue or binding failed.
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
    /// The publish was rejected before reaching the broker.
    #[error(
        "failed to publish to exchange '{exchange}' with routing key '{routing_key}': {source}"
    )]
    Publish {
        /// Target exchange.
        exchange: String,
        /// Target routing key.
        routing_key: String,
        /// Underlying lapin error (boxed: it dwarfs every other field).
        #[source]
        source: Box<lapin::Error>,
    },
    /// The broker negatively acknowledged the publish.
    #[error(
        "the broker negatively acknowledged the publish to exchange '{exchange}' with routing key '{routing_key}'"
    )]
    Nack {
        /// Target exchange.
        exchange: String,
        /// Target routing key.
        routing_key: String,
    },
    /// The broker sent no confirmation (confirm mode was not active).
    #[error(
        "the broker did not confirm the publish to exchange '{exchange}' with routing key '{routing_key}'"
    )]
    Unconfirmed {
        /// Target exchange.
        exchange: String,
        /// Target routing key.
        routing_key: String,
    },
    /// Closing the channel or connection failed.
    #[error("failed to close the AMQP connection: {source}")]
    Close {
        /// Underlying lapin error (boxed: it dwarfs every other field).
        #[source]
        source: Box<lapin::Error>,
    },
}

impl AmqpError {
    fn connect(config: &RabbitConfig, source: lapin::Error) -> Self {
        Self::Connect {
            host: config.host.clone(),
            port: config.port,
            source: Box::new(source),
        }
    }

    fn declare(kind: &'static str, name: &str, source: lapin::Error) -> Self {
        Self::Declare {
            kind,
            name: name.to_owned(),
            source: Box::new(source),
        }
    }

    fn publish(exchange: &str, routing_key: &str, source: lapin::Error) -> Self {
        Self::Publish {
            exchange: exchange.to_owned(),
            routing_key: routing_key.to_owned(),
            source: Box::new(source),
        }
    }

    fn nack(exchange: &str, routing_key: &str) -> Self {
        Self::Nack {
            exchange: exchange.to_owned(),
            routing_key: routing_key.to_owned(),
        }
    }

    fn unconfirmed(exchange: &str, routing_key: &str) -> Self {
        Self::Unconfirmed {
            exchange: exchange.to_owned(),
            routing_key: routing_key.to_owned(),
        }
    }

    fn close(source: lapin::Error) -> Self {
        Self::Close {
            source: Box::new(source),
        }
    }
}

/// One AMQP connection (auto-recovery enabled) and one publish channel in
/// confirm mode.
pub struct AmqpClient {
    connection: Connection,
    publish_channel: Channel,
}

impl AmqpClient {
    /// Connects and puts the publish channel in confirm mode.
    ///
    /// Callers gate on [`RabbitConfig::enabled`]; this function always dials.
    pub async fn connect(config: &RabbitConfig) -> Result<Self, AmqpError> {
        let properties = ConnectionProperties::default()
            .enable_auto_recover()
            .with_connection_name(crate::APP_NAME.into());
        let connection = Connection::connect(&config.address(), properties)
            .await
            .map_err(|source| AmqpError::connect(config, source))?;
        let publish_channel =
            connection
                .create_channel()
                .await
                .map_err(|source| AmqpError::OpenChannel {
                    source: Box::new(source),
                })?;
        publish_channel
            .confirm_select(ConfirmSelectOptions::default())
            .await
            .map_err(|source| AmqpError::EnableConfirms {
                source: Box::new(source),
            })?;

        Ok(Self {
            connection,
            publish_channel,
        })
    }

    /// The underlying connection, for deployments that need extra channels
    /// (ticket #9's consumer path opens its own channel here; cancel first,
    /// close after).
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Declares the full frozen topology: the `main` and `deadLetter`
    /// exchanges, `task_queue` and its `task.#` binding. Idempotent;
    /// auto-recovery replays it after a reconnect.
    pub async fn declare_topology(&self) -> Result<(), AmqpError> {
        self.publisher().declare_exchange(MAIN_EXCHANGE).await?;
        self.publisher()
            .declare_exchange(DEAD_LETTER_EXCHANGE)
            .await?;

        self.publish_channel
            .queue_declare(
                TASK_QUEUE.into(),
                QueueDeclareOptions {
                    durable: true,
                    ..Default::default()
                },
                queue_arguments(),
            )
            .await
            .map_err(|source| AmqpError::declare("queue", TASK_QUEUE, source))?;

        self.publish_channel
            .queue_bind(
                TASK_QUEUE.into(),
                MAIN_EXCHANGE.into(),
                TASK_BINDING_KEY.into(),
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await
            .map_err(|source| AmqpError::declare("binding", TASK_BINDING_KEY, source))?;

        tracing::info!("RabbitMQ topology declared");
        Ok(())
    }

    /// Publishes `payload` to `exchange` with `routing_key` and awaits the
    /// broker confirm (confirm mode was enabled at connect).
    ///
    /// The owning client's publish entry point: tests and services publish
    /// through it while they hold the client. [`AmqpClient::publisher`] hands
    /// the same channel out as an owned handle for components that outlive a
    /// borrow; see [`Publisher::publish`] for the failure semantics.
    pub async fn publish(
        &self,
        exchange: &str,
        routing_key: &str,
        payload: &[u8],
    ) -> Result<Confirmation, AmqpError> {
        self.publisher()
            .publish(exchange, routing_key, payload)
            .await
    }

    /// A cheap, cloneable handle to the publish channel (confirm mode), for
    /// components that outlive a borrow of this client — ticket #9's consumer
    /// serializes it into its consume loop to republish handler failures.
    pub fn publisher(&self) -> Publisher {
        Publisher {
            channel: self.publish_channel.clone(),
        }
    }

    /// Closes the publish channel, then the connection (research §5 order:
    /// consumers cancel first; this client has none).
    ///
    /// Both steps are attempted; the first error — if any — is returned.
    pub async fn close(&self) -> Result<(), AmqpError> {
        let channel_result = self.publish_channel.close(200, "shutdown".into()).await;
        let connection_result = self.connection.close(200, "shutdown".into()).await;
        channel_result
            .and(connection_result)
            .map_err(AmqpError::close)
    }
}

/// A cloneable handle to a confirm-mode publish channel
/// ([`AmqpClient::publisher`]).
#[derive(Clone)]
pub struct Publisher {
    channel: Channel,
}

impl Publisher {
    /// Publishes `payload` to `exchange` with `routing_key` and awaits the
    /// broker confirm.
    ///
    /// The exchange is declared on the publish path, mirroring the TS
    /// wrapper. Any failure — declaration, publish, nack or a missing confirm
    /// — is returned as [`AmqpError`]; following the Fastify convention,
    /// **callers log and continue** instead of aborting the request or the
    /// process.
    pub async fn publish(
        &self,
        exchange: &str,
        routing_key: &str,
        payload: &[u8],
    ) -> Result<Confirmation, AmqpError> {
        self.declare_exchange(exchange).await?;

        let confirmation = self
            .channel
            .basic_publish(
                exchange.into(),
                routing_key.into(),
                BasicPublishOptions::default(),
                payload,
                BasicProperties::default(),
            )
            .await
            .map_err(|source| AmqpError::publish(exchange, routing_key, source))?
            .await
            .map_err(|source| AmqpError::publish(exchange, routing_key, source))?;

        match confirmation {
            Confirmation::Ack(_) => {
                tracing::info!(
                    exchange,
                    routing_key,
                    "Successfully published RabbitMQ message"
                );
                Ok(confirmation)
            }
            Confirmation::Nack(_) => Err(AmqpError::nack(exchange, routing_key)),
            Confirmation::NotRequested => Err(AmqpError::unconfirmed(exchange, routing_key)),
        }
    }

    /// Declares `exchange` with its frozen definition (`main` carries the
    /// alternate-exchange and 24h-TTL arguments; `deadLetter` and any other
    /// exchange are plain durable topic exchanges). Idempotent.
    async fn declare_exchange(&self, exchange: &str) -> Result<(), AmqpError> {
        self.channel
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
            .map_err(|source| AmqpError::declare("exchange", exchange, source))?;
        Ok(())
    }
}

/// Connects and declares the full frozen topology: the startup path for
/// publisher and consumer deployments alike.
pub async fn connect_and_declare(config: &RabbitConfig) -> Result<AmqpClient, AmqpError> {
    let client = AmqpClient::connect(config).await?;
    client.declare_topology().await?;
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RabbitTransport;

    fn lapin_error() -> lapin::Error {
        lapin::Error::from(lapin::ErrorKind::InvalidChannel(0))
    }

    fn config() -> RabbitConfig {
        RabbitConfig {
            enabled: true,
            host: "rabbit.internal".to_owned(),
            port: 5673,
            user: "user".to_owned(),
            password: "hunter2".to_owned(),
            transport: RabbitTransport::Amqp,
        }
    }

    #[test]
    fn connect_errors_name_the_broker_without_the_password() {
        let message = AmqpError::connect(&config(), lapin_error()).to_string();

        assert!(message.contains("rabbit.internal:5673"), "{message}");
        assert!(!message.contains("hunter2"), "{message}");
    }

    #[test]
    fn publish_errors_name_the_exchange_and_routing_key() {
        let message = AmqpError::publish("main", "task.7", lapin_error()).to_string();

        assert!(message.contains("'main'"), "{message}");
        assert!(message.contains("'task.7'"), "{message}");
    }

    #[test]
    fn declaration_errors_name_the_kind_and_subject() {
        let message = AmqpError::declare("queue", "task_queue", lapin_error()).to_string();

        assert!(message.contains("queue 'task_queue'"), "{message}");
    }

    #[test]
    fn nacked_and_unconfirmed_publishes_are_errors() {
        let nacked = AmqpError::nack("main", "task.7").to_string();
        assert!(nacked.contains("negatively acknowledged"), "{nacked}");

        let unconfirmed = AmqpError::unconfirmed("main", "task.7").to_string();
        assert!(unconfirmed.contains("did not confirm"), "{unconfirmed}");
    }

    #[test]
    fn close_errors_name_the_close() {
        let message = AmqpError::close(lapin_error()).to_string();

        assert!(message.contains("close the AMQP connection"), "{message}");
    }
}
