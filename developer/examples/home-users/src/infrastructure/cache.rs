//! Valkey cache client, ported from the Fastify template's
//! `src/infrastructure/cache.ts`.
//!
//! Semantics:
//!
//! - The gate is exact-match (see [`crate::config::CacheConfig`]): a disabled
//!   client performs no network traffic at all, and every operation on an
//!   uninitialized client fails with `"cache client not initialized"`.
//! - When enabled, the connection is established at startup; a connection
//!   failure is returned to the caller so the service app can exit (Fastify's
//!   `process.exit(1)`).
//! - Key namespacing and TTLs are per-service concerns; this module exposes
//!   plain `GET`/`SET`/`DEL` only.

use redis::aio::MultiplexedConnection;
use redis::{
    AsyncCommands, Client, ConnectionAddr, ConnectionInfo, IntoConnectionInfo, RedisConnectionInfo,
};
use thiserror::Error;

use crate::config::CacheConfig;

/// Errors produced by [`CacheClient`].
#[derive(Debug, Error)]
pub enum CacheError {
    /// An operation ran before a successful [`CacheClient::connect`], including
    /// when the cache is disabled (`"cache client not initialized"`, parity).
    #[error("cache client not initialized")]
    NotInitialized,
    /// The startup connection attempt failed.
    #[error("failed to connect to cache at {host}:{port}: {source}")]
    Connect {
        /// Host the client tried to reach.
        host: String,
        /// Port the client tried to reach.
        port: u16,
        /// Underlying driver error.
        source: redis::RedisError,
    },
    /// A command failed on an established connection.
    #[error("cache command failed: {source}")]
    Command {
        /// Underlying driver error.
        source: redis::RedisError,
    },
}

impl CacheError {
    fn command(source: redis::RedisError) -> Self {
        Self::Command { source }
    }

    fn connect(config: &CacheConfig, source: redis::RedisError) -> Self {
        Self::Connect {
            host: config.host.clone(),
            port: config.port,
            source,
        }
    }
}

/// A cheaply cloneable handle to the cache connection.
///
/// Clones share one multiplexed connection, so the handle can live in
/// [`crate::server::AppState`] and in the shutdown cleanup registry at once.
/// The connection closes once the last clone is dropped.
#[derive(Clone, Default)]
pub struct CacheClient {
    connection: Option<MultiplexedConnection>,
}

impl std::fmt::Debug for CacheClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CacheClient")
            .field("initialized", &self.is_initialized())
            .finish()
    }
}

impl CacheClient {
    /// An uninitialized handle: every operation returns
    /// [`CacheError::NotInitialized`].
    pub fn uninitialized() -> Self {
        Self::default()
    }

    /// Whether a connection has been established.
    pub fn is_initialized(&self) -> bool {
        self.connection.is_some()
    }

    /// Creates the client: when the config is disabled this returns an
    /// uninitialized handle without touching the network; otherwise it connects
    /// and fails fast with [`CacheError::Connect`].
    pub async fn connect(config: &CacheConfig) -> Result<Self, CacheError> {
        if config.disabled {
            tracing::info!("cache client disabled; skipping connect");
            return Ok(Self::uninitialized());
        }

        let client = Client::open(connection_info(config))
            .map_err(|source| CacheError::connect(config, source))?;
        let connection = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|source| CacheError::connect(config, source))?;

        tracing::info!(host = %config.host, port = config.port, "cache client connected");
        Ok(Self {
            connection: Some(connection),
        })
    }

    /// `GET key`: `Ok(None)` when the key does not exist.
    pub async fn get(&self, key: &str) -> Result<Option<String>, CacheError> {
        let mut connection = self.connection()?.clone();
        connection.get(key).await.map_err(CacheError::command)
    }

    /// `SET key value`.
    pub async fn set(&self, key: &str, value: &str) -> Result<(), CacheError> {
        let mut connection = self.connection()?.clone();
        connection
            .set(key, value)
            .await
            .map_err(CacheError::command)
    }

    /// `DEL key`, returning the number of deleted keys.
    pub async fn del(&self, key: &str) -> Result<usize, CacheError> {
        let mut connection = self.connection()?.clone();
        connection.del(key).await.map_err(CacheError::command)
    }

    /// Drops this handle's connection; the socket closes when the last clone
    /// goes away.
    pub fn close(&mut self) {
        if self.connection.take().is_some() {
            tracing::info!("cache connection closed");
        }
    }

    fn connection(&self) -> Result<&MultiplexedConnection, CacheError> {
        self.connection.as_ref().ok_or(CacheError::NotInitialized)
    }
}

/// Builds the driver connection info. An empty password means no `AUTH` is sent
/// (node-redis parity); any password is sent as-is.
fn connection_info(config: &CacheConfig) -> ConnectionInfo {
    let redis_settings = if config.password.is_empty() {
        RedisConnectionInfo::default()
    } else {
        RedisConnectionInfo::default().set_password(&config.password)
    };

    ConnectionAddr::Tcp(config.host.clone(), config.port)
        .into_connection_info()
        .expect("a TCP address always converts to connection info")
        .set_redis_settings(redis_settings)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn disabled_config() -> CacheConfig {
        CacheConfig {
            disabled: true,
            // `.invalid` never resolves (RFC 2606): if the client attempted to
            // connect, it would fail instead of returning Ok.
            host: "no-such-host.invalid".to_owned(),
            port: 6379,
            password: String::new(),
        }
    }

    #[tokio::test]
    async fn disabled_cache_is_uninitialized_and_touches_no_network() {
        let client = tokio::time::timeout(
            Duration::from_secs(1),
            CacheClient::connect(&disabled_config()),
        )
        .await
        .expect("a disabled client must not perform any I/O")
        .expect("a disabled client still creates a usable handle");

        assert!(!client.is_initialized());
    }

    #[tokio::test]
    async fn uninitialized_operations_return_the_not_initialized_error() {
        let clients = [
            CacheClient::default(),
            CacheClient::connect(&disabled_config()).await.unwrap(),
        ];

        for client in &clients {
            let error = client.get("key").await.unwrap_err();
            assert!(matches!(error, CacheError::NotInitialized), "{error:?}");
            assert_eq!(error.to_string(), "cache client not initialized");

            let error = client.set("key", "value").await.unwrap_err();
            assert!(matches!(error, CacheError::NotInitialized), "{error:?}");

            let error = client.del("key").await.unwrap_err();
            assert!(matches!(error, CacheError::NotInitialized), "{error:?}");
        }
    }

    #[tokio::test]
    async fn enabled_cache_fails_fast_when_the_server_is_unreachable() {
        // Reserve a port and release it, so connecting is refused immediately.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let config = CacheConfig {
            disabled: false,
            host: "127.0.0.1".to_owned(),
            port,
            password: String::new(),
        };

        let error = tokio::time::timeout(Duration::from_secs(5), CacheClient::connect(&config))
            .await
            .expect("connect must fail fast, not hang")
            .expect_err("connecting to a dead port must fail");

        match &error {
            CacheError::Connect {
                host,
                port: failed_port,
                ..
            } => {
                assert_eq!(host, "127.0.0.1");
                assert_eq!(*failed_port, port);
            }
            other => panic!("expected CacheError::Connect, got {other:?}"),
        }
        let message = error.to_string();
        assert!(message.contains("127.0.0.1"), "{message}");
        assert!(message.contains(&port.to_string()), "{message}");
    }
}
