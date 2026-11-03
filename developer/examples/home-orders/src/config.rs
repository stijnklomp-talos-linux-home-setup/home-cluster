//! Typed application configuration.
//!
//! Loading order, highest precedence first:
//!
//! 1. the real process environment;
//! 2. `.env` (personal, gitignored overrides);
//! 3. `.env.{APP_ENV}` (committed environment defaults, e.g. `.env.development`);
//! 4. built-in defaults.
//!
//! `APP_ENV` is resolved from the process environment and `.env` only (the
//! environment-specific file is selected by it, so it cannot define it), then
//! defaults to `development`.
//!
//! Files are parsed with dotenvy's parser, but values stay in a local overlay
//! instead of being written into the process environment: writing the process
//! environment is process-global and unsafe in concurrent programs, and the
//! typed struct is the only config every component reads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::level_filters::LevelFilter;

/// Environment the service runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppEnv {
    /// Local development.
    #[default]
    Development,
    /// Production.
    Production,
    /// Test runs.
    Tests,
}

impl AppEnv {
    /// Canonical lowercase name, used in env files and logs.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
            Self::Tests => "tests",
        }
    }

    /// Whether this is the production environment.
    pub const fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "development" => Some(Self::Development),
            "production" => Some(Self::Production),
            "tests" => Some(Self::Tests),
            _ => None,
        }
    }
}

/// `APP_ENV` default, matching the Fastify template.
pub const DEFAULT_APP_ENV: AppEnv = AppEnv::Development;
/// `API_PORT` default, matching the Fastify template.
pub const DEFAULT_API_PORT: u16 = 3000;
/// `REQUEST_LOG_LEVEL` / `APP_LOG_LEVEL` default, matching the Fastify template.
pub const DEFAULT_LOG_LEVEL: LevelFilter = LevelFilter::INFO;
/// `RABBIT_HOST` default, matching the Fastify template.
pub const DEFAULT_RABBIT_HOST: &str = "0.0.0.0";
/// `RABBIT_PORT` default. The Fastify code defaulted to 5671 (a bug — every
/// env file sets 5672); the Rust template defaults to the corrected 5672.
pub const DEFAULT_RABBIT_PORT: u16 = 5672;
/// `RABBIT_USER` default, matching the Fastify template.
pub const DEFAULT_RABBIT_USER: &str = "guest";
/// `RABBIT_PASSWORD` default, matching the Fastify template.
pub const DEFAULT_RABBIT_PASSWORD: &str = "guest";
/// Whether RabbitMQ starts enabled when `RABBIT_DISABLED` is unset.
pub const RABBIT_ENABLED_WHEN_UNSET: bool = true;
/// `CACHE_HOST` default, matching the Fastify template.
pub const DEFAULT_CACHE_HOST: &str = "localhost";
/// `CACHE_PORT` default, matching the Fastify template.
pub const DEFAULT_CACHE_PORT: u16 = 6379;
/// `CACHE_PASSWORD` default, matching the Fastify template: empty, no `AUTH`.
pub const DEFAULT_CACHE_PASSWORD: &str = "";
/// `WORKER_QUEUE` default, matching the Fastify template.
pub const DEFAULT_WORKER_QUEUE: &str = "task_queue";
/// `WORKER_EXCHANGE` default, matching the Fastify template.
pub const DEFAULT_WORKER_EXCHANGE: &str = "main";
/// `WORKER_BINDING_KEY` default, matching the Fastify template.
pub const DEFAULT_WORKER_BINDING_KEY: &str = "task.#";
/// `WORKER_PREFETCH` default: 1, a Rust-side addition (ADR-0006; the TS
/// wrapper had its prefetch call commented out).
pub const DEFAULT_WORKER_PREFETCH: u16 = 1;
/// `GRAPHQL_ENABLED` default when the `graphql` feature is compiled in
/// (ADR-0011): expose the subgraph unless an environment turns it off.
#[cfg(feature = "graphql")]
pub const DEFAULT_GRAPHQL_ENABLED: bool = true;
/// `GRAPHQL_JWKS_URL` default: a local Keycloak-style endpoint for development;
/// deployments point this at the identity provider's JWKS.
#[cfg(feature = "graphql")]
pub const DEFAULT_GRAPHQL_JWKS_URL: &str =
    "http://localhost:8080/realms/home/protocol/openid-connect/certs";
/// `GRAPHQL_JWT_ISSUER` default, matching [`DEFAULT_GRAPHQL_JWKS_URL`].
#[cfg(feature = "graphql")]
pub const DEFAULT_GRAPHQL_JWT_ISSUER: &str = "http://localhost:8080/realms/home";
/// `GRAPHQL_JWT_AUDIENCE` default: the service's own package name. Set the
/// variable to an empty string to disable the `aud` check (Cognito access
/// tokens carry `client_id`, not `aud`).
#[cfg(feature = "graphql")]
pub const DEFAULT_GRAPHQL_JWT_AUDIENCE: &str = crate::APP_NAME;

/// Everything the service app reads from the environment at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    /// Deployment environment ([`AppEnv`]).
    pub app_env: AppEnv,
    /// Port the HTTP server binds (`API_PORT`).
    pub api_port: u16,
    /// Postgres connection URL (`DATABASE_URL`), required.
    pub database_url: String,
    /// Verbosity of request logs (`REQUEST_LOG_LEVEL`).
    pub request_log_level: LevelFilter,
    /// Verbosity of all other logs (`APP_LOG_LEVEL`).
    pub app_log_level: LevelFilter,
    /// Cache (Valkey) connection settings (`CACHE_*`).
    pub cache: CacheConfig,
    /// RabbitMQ client configuration (`RABBIT_*`).
    pub rabbit: RabbitConfig,
    /// Worker (AMQP consumer) settings (`WORKER_*`), read by dedicated
    /// consumer deployments; the service binary itself ignores them.
    pub worker: WorkerConfig,
    /// Optional GraphQL federation subgraph (`GRAPHQL_*`, feature `graphql`).
    #[cfg(feature = "graphql")]
    pub graphql: GraphQLConfig,
}

/// GraphQL federation subgraph settings (`GRAPHQL_*`), see ADR-0011.
#[cfg(feature = "graphql")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphQLConfig {
    /// `GRAPHQL_ENABLED`: expose `/graphql`; unset enables it.
    pub enabled: bool,
    /// `GRAPHQL_JWKS_URL`: JWKS endpoint used to verify bearer tokens.
    pub jwks_url: String,
    /// `GRAPHQL_JWT_ISSUER`: required `iss` claim.
    pub issuer: String,
    /// `GRAPHQL_JWT_AUDIENCE`: required `aud` claim. `None` disables the
    /// audience check, which is required for Cognito *access* tokens (they
    /// carry `client_id` and no `aud`).
    pub audience: Option<String>,
    /// `GRAPHQL_JWT_CLIENT_ID`: required `client_id` claim, the Cognito
    /// access-token equivalent of `aud`. `None` disables the check.
    pub client_id: Option<String>,
}

/// Cache (Valkey) connection settings, ported from the Fastify template's
/// `src/infrastructure/cache.ts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheConfig {
    /// `CACHE_DISABLED`: only unset or the exact string `"false"` enables the
    /// cache; every other value disables it (exact match, Fastify parity).
    pub disabled: bool,
    /// `CACHE_HOST`, defaulting to `localhost`.
    pub host: String,
    /// `CACHE_PORT`, defaulting to `6379`.
    pub port: u16,
    /// `CACHE_PASSWORD`, defaulting to empty (no `AUTH` is sent).
    pub password: String,
}

/// RabbitMQ transport selected by `RABBIT_TRANSPORT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RabbitTransport {
    /// Plain `amqp://` transport.
    #[default]
    Amqp,
    /// TLS `amqps://` transport.
    Amqps,
}

impl RabbitTransport {
    /// URI scheme used when connecting.
    pub const fn scheme(self) -> &'static str {
        match self {
            Self::Amqp => "amqp",
            Self::Amqps => "amqps",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "amqp" => Some(Self::Amqp),
            "amqps" => Some(Self::Amqps),
            _ => None,
        }
    }
}

/// RabbitMQ client configuration (`RABBIT_*`).
///
/// `enabled` implements the frozen gate: `RABBIT_DISABLED` is compared as an
/// exact string — unset or `"false"` enables the client, anything else
/// (including `"False"`, `"1"` or `""`) disables it, and then no connection is
/// attempted at all.
#[derive(Clone, PartialEq, Eq)]
pub struct RabbitConfig {
    /// Whether the RabbitMQ client runs (`RABBIT_DISABLED` gate).
    pub enabled: bool,
    /// Broker host (`RABBIT_HOST`).
    pub host: String,
    /// Broker port (`RABBIT_PORT`).
    pub port: u16,
    /// Broker user (`RABBIT_USER`).
    pub user: String,
    /// Broker password (`RABBIT_PASSWORD`); redacted from [`Debug`].
    pub password: String,
    /// Broker transport (`RABBIT_TRANSPORT`).
    pub transport: RabbitTransport,
}

impl RabbitConfig {
    /// Broker address for the configured transport.
    pub fn address(&self) -> String {
        format!(
            "{}://{}:{}@{}:{}",
            self.transport.scheme(),
            self.user,
            self.password,
            self.host,
            self.port
        )
    }
}

impl std::fmt::Debug for RabbitConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RabbitConfig")
            .field("enabled", &self.enabled)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &"***")
            .field("transport", &self.transport)
            .finish()
    }
}

/// Worker (AMQP consumer) settings (`WORKER_*`).
///
/// Ported from the Fastify template's `src/worker/index.ts`: queue, exchange
/// and binding key are configurable with the same defaults; `prefetch` is the
/// Rust-side replacement for the TS wrapper's commented-out `prefetch(1)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    /// Queue consumed from (`WORKER_QUEUE`), default `task_queue`.
    pub queue: String,
    /// Exchange the queue is bound to (`WORKER_EXCHANGE`), default `main`.
    pub exchange: String,
    /// Binding key of the queue on `exchange` (`WORKER_BINDING_KEY`), default
    /// `task.#`.
    pub binding_key: String,
    /// Maximum unacknowledged messages per consumer (`WORKER_PREFETCH`),
    /// default 1. `0` means unlimited, per AMQP.
    pub prefetch: u16,
}

/// Configuration loading or validation error.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// An env file exists but could not be read or parsed.
    #[error("failed to read {path}: {message}")]
    EnvFile {
        /// File that could not be read.
        path: PathBuf,
        /// Parser or IO error.
        message: String,
    },
    /// A value was present but not one of the accepted values.
    #[error("invalid value for {key}: {value:?} (expected {expected})")]
    InvalidValue {
        /// Environment variable name.
        key: &'static str,
        /// Rejected value.
        value: String,
        /// Human-readable description of the accepted values.
        expected: &'static str,
    },
    /// A required variable was not set at all.
    #[error("missing required environment variable {key} (expected {expected})")]
    Missing {
        /// Environment variable name.
        key: &'static str,
        /// Human-readable description of the accepted values.
        expected: &'static str,
    },
}

const ENV_APP_ENV: &str = "APP_ENV";
const ENV_API_PORT: &str = "API_PORT";
const ENV_DATABASE_URL: &str = "DATABASE_URL";
const ENV_REQUEST_LOG_LEVEL: &str = "REQUEST_LOG_LEVEL";
const ENV_APP_LOG_LEVEL: &str = "APP_LOG_LEVEL";
const ENV_RABBIT_DISABLED: &str = "RABBIT_DISABLED";
const ENV_RABBIT_HOST: &str = "RABBIT_HOST";
const ENV_RABBIT_PORT: &str = "RABBIT_PORT";
const ENV_RABBIT_USER: &str = "RABBIT_USER";
const ENV_RABBIT_PASSWORD: &str = "RABBIT_PASSWORD";
const ENV_RABBIT_TRANSPORT: &str = "RABBIT_TRANSPORT";
const ENV_CACHE_DISABLED: &str = "CACHE_DISABLED";
const ENV_CACHE_HOST: &str = "CACHE_HOST";
const ENV_CACHE_PORT: &str = "CACHE_PORT";
const ENV_CACHE_PASSWORD: &str = "CACHE_PASSWORD";
const ENV_WORKER_QUEUE: &str = "WORKER_QUEUE";
const ENV_WORKER_EXCHANGE: &str = "WORKER_EXCHANGE";
const ENV_WORKER_BINDING_KEY: &str = "WORKER_BINDING_KEY";
const ENV_WORKER_PREFETCH: &str = "WORKER_PREFETCH";
#[cfg(feature = "graphql")]
const ENV_GRAPHQL_ENABLED: &str = "GRAPHQL_ENABLED";
#[cfg(feature = "graphql")]
const ENV_GRAPHQL_JWKS_URL: &str = "GRAPHQL_JWKS_URL";
#[cfg(feature = "graphql")]
const ENV_GRAPHQL_JWT_ISSUER: &str = "GRAPHQL_JWT_ISSUER";
#[cfg(feature = "graphql")]
const ENV_GRAPHQL_JWT_AUDIENCE: &str = "GRAPHQL_JWT_AUDIENCE";
#[cfg(feature = "graphql")]
const ENV_GRAPHQL_JWT_CLIENT_ID: &str = "GRAPHQL_JWT_CLIENT_ID";

const APP_ENV_VALUES: &str = "development, production or tests";
const DATABASE_URL_VALUES: &str = "a non-empty postgres:// connection URL";
const LOG_LEVEL_VALUES: &str = "trace, debug, info, warn, error or off";
const PORT_VALUES: &str = "an integer between 0 and 65535";
const RABBIT_TRANSPORT_VALUES: &str = "amqp or amqps";
#[cfg(feature = "graphql")]
const BOOLEAN_VALUES: &str = "true or false";
#[cfg(feature = "graphql")]
const HTTP_URL_VALUES: &str = "a non-empty http:// or https:// URL";

impl AppConfig {
    /// Loads configuration from the working directory and process environment.
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(Path::new("."), |key| std::env::var(key).ok())
    }

    /// Loads configuration from `dir` (`.env` and `.env.{APP_ENV}`) overlaid by
    /// `process_env`. Exposed for tests and for services with custom layouts.
    pub fn load_from(
        dir: &Path,
        process_env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let personal = read_env_file(&dir.join(".env"))?;
        let app_env = process_env(ENV_APP_ENV)
            .or_else(|| personal.get(ENV_APP_ENV).cloned())
            .unwrap_or_else(|| DEFAULT_APP_ENV.as_str().to_owned());

        // `.env` (personal overrides) wins over `.env.{APP_ENV}`, mirroring
        // dotenvy's first-load-wins semantics for non-overriding loads.
        let mut overlay = read_env_file(&dir.join(format!(".env.{app_env}")))?;
        overlay.extend(personal);

        Self::from_lookup(|key| process_env(key).or_else(|| overlay.get(key).cloned()))
    }

    /// Parses configuration from a single `key -> value` lookup, applying defaults.
    pub fn from_lookup(env: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let app_env_value = env(ENV_APP_ENV).unwrap_or_else(|| DEFAULT_APP_ENV.as_str().to_owned());
        let app_env = AppEnv::parse(&app_env_value).ok_or(ConfigError::InvalidValue {
            key: ENV_APP_ENV,
            value: app_env_value,
            expected: APP_ENV_VALUES,
        })?;

        let api_port = parse_u16(ENV_API_PORT, env(ENV_API_PORT), DEFAULT_API_PORT)?;

        let database_url = match env(ENV_DATABASE_URL) {
            Some(value) if !value.trim().is_empty() => value.trim().to_owned(),
            Some(value) => {
                return Err(ConfigError::InvalidValue {
                    key: ENV_DATABASE_URL,
                    value,
                    expected: DATABASE_URL_VALUES,
                });
            }
            None => {
                return Err(ConfigError::Missing {
                    key: ENV_DATABASE_URL,
                    expected: DATABASE_URL_VALUES,
                });
            }
        };

        Ok(Self {
            app_env,
            api_port,
            database_url,
            request_log_level: parse_log_level(ENV_REQUEST_LOG_LEVEL, env(ENV_REQUEST_LOG_LEVEL))?,
            app_log_level: parse_log_level(ENV_APP_LOG_LEVEL, env(ENV_APP_LOG_LEVEL))?,
            cache: parse_cache(&env)?,
            rabbit: rabbit_from_lookup(&env)?,
            worker: worker_from_lookup(&env)?,
            #[cfg(feature = "graphql")]
            graphql: graphql_from_lookup(&env)?,
        })
    }
}

/// Parses an optional numeric `u16` variable (ports, prefetch), trimming
/// whitespace; a present but unparsable value is an [`ConfigError::InvalidValue`].
fn parse_u16(key: &'static str, value: Option<String>, default: u16) -> Result<u16, ConfigError> {
    match value {
        Some(value) => value
            .trim()
            .parse::<u16>()
            .map_err(|_| ConfigError::InvalidValue {
                key,
                value,
                expected: PORT_VALUES,
            }),
        None => Ok(default),
    }
}

/// Parses the `CACHE_*` variables, applying the Fastify defaults.
fn parse_cache(env: &impl Fn(&str) -> Option<String>) -> Result<CacheConfig, ConfigError> {
    let port = parse_u16(ENV_CACHE_PORT, env(ENV_CACHE_PORT), DEFAULT_CACHE_PORT)?;

    Ok(CacheConfig {
        disabled: parse_cache_disabled(env(ENV_CACHE_DISABLED)),
        host: env(ENV_CACHE_HOST).unwrap_or_else(|| DEFAULT_CACHE_HOST.to_owned()),
        port,
        password: env(ENV_CACHE_PASSWORD).unwrap_or_else(|| DEFAULT_CACHE_PASSWORD.to_owned()),
    })
}

/// `CACHE_DISABLED` gate, exact Fastify parity: unset or the exact string
/// `"false"` enables the cache; any other value disables it.
fn parse_cache_disabled(value: Option<String>) -> bool {
    match value {
        Some(value) => value != "false",
        None => false,
    }
}

/// Parses the `RABBIT_*` block with the frozen `RABBIT_DISABLED` semantics:
/// `process.env.RABBIT_DISABLED ?? "false"` compared against the exact string
/// `"false"` (no trimming, no case folding).
fn rabbit_from_lookup(env: &impl Fn(&str) -> Option<String>) -> Result<RabbitConfig, ConfigError> {
    let enabled = env(ENV_RABBIT_DISABLED)
        .map(|value| value == "false")
        .unwrap_or(RABBIT_ENABLED_WHEN_UNSET);

    let port = parse_u16(ENV_RABBIT_PORT, env(ENV_RABBIT_PORT), DEFAULT_RABBIT_PORT)?;

    let transport = match env(ENV_RABBIT_TRANSPORT) {
        Some(value) => RabbitTransport::parse(&value.trim().to_ascii_lowercase()).ok_or(
            ConfigError::InvalidValue {
                key: ENV_RABBIT_TRANSPORT,
                value,
                expected: RABBIT_TRANSPORT_VALUES,
            },
        )?,
        None => RabbitTransport::default(),
    };

    Ok(RabbitConfig {
        enabled,
        host: env(ENV_RABBIT_HOST).unwrap_or_else(|| DEFAULT_RABBIT_HOST.to_owned()),
        port,
        user: env(ENV_RABBIT_USER).unwrap_or_else(|| DEFAULT_RABBIT_USER.to_owned()),
        password: env(ENV_RABBIT_PASSWORD).unwrap_or_else(|| DEFAULT_RABBIT_PASSWORD.to_owned()),
        transport,
    })
}

/// Parses the `WORKER_*` block with the Fastify template's defaults.
fn worker_from_lookup(env: &impl Fn(&str) -> Option<String>) -> Result<WorkerConfig, ConfigError> {
    let prefetch = parse_u16(
        ENV_WORKER_PREFETCH,
        env(ENV_WORKER_PREFETCH),
        DEFAULT_WORKER_PREFETCH,
    )?;

    Ok(WorkerConfig {
        queue: env(ENV_WORKER_QUEUE).unwrap_or_else(|| DEFAULT_WORKER_QUEUE.to_owned()),
        exchange: env(ENV_WORKER_EXCHANGE).unwrap_or_else(|| DEFAULT_WORKER_EXCHANGE.to_owned()),
        binding_key: env(ENV_WORKER_BINDING_KEY)
            .unwrap_or_else(|| DEFAULT_WORKER_BINDING_KEY.to_owned()),
        prefetch,
    })
}

/// Parses the `GRAPHQL_*` block (feature `graphql`, ADR-0011).
#[cfg(feature = "graphql")]
fn graphql_from_lookup(
    env: &impl Fn(&str) -> Option<String>,
) -> Result<GraphQLConfig, ConfigError> {
    let enabled = match env(ENV_GRAPHQL_ENABLED) {
        Some(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" => true,
            "false" => false,
            _ => {
                return Err(ConfigError::InvalidValue {
                    key: ENV_GRAPHQL_ENABLED,
                    value,
                    expected: BOOLEAN_VALUES,
                });
            }
        },
        None => DEFAULT_GRAPHQL_ENABLED,
    };

    let jwks_url = http_url(
        ENV_GRAPHQL_JWKS_URL,
        env(ENV_GRAPHQL_JWKS_URL),
        DEFAULT_GRAPHQL_JWKS_URL,
    )?;
    let issuer = http_url(
        ENV_GRAPHQL_JWT_ISSUER,
        env(ENV_GRAPHQL_JWT_ISSUER),
        DEFAULT_GRAPHQL_JWT_ISSUER,
    )?;
    // An empty value disables the check: Cognito access tokens have no
    // `aud`. Unset keeps the development/test default.
    let audience = match env(ENV_GRAPHQL_JWT_AUDIENCE) {
        Some(value) => {
            let value = value.trim().to_owned();
            (!value.is_empty()).then_some(value)
        }
        None => Some(DEFAULT_GRAPHQL_JWT_AUDIENCE.to_owned()),
    };
    let client_id = env(ENV_GRAPHQL_JWT_CLIENT_ID)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());

    Ok(GraphQLConfig {
        enabled,
        jwks_url,
        issuer,
        audience,
        client_id,
    })
}

/// Parses a non-empty `http://`/`https://` setting with a host, applying
/// `default` when unset.
#[cfg(feature = "graphql")]
fn http_url(
    key: &'static str,
    value: Option<String>,
    default: &'static str,
) -> Result<String, ConfigError> {
    let value = value.unwrap_or_else(|| default.to_owned());
    let value = value.trim().to_owned();
    let valid = reqwest::Url::parse(&value)
        .map(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some())
        .unwrap_or(false);
    if valid {
        Ok(value)
    } else {
        Err(ConfigError::InvalidValue {
            key,
            value,
            expected: HTTP_URL_VALUES,
        })
    }
}

fn parse_log_level(key: &'static str, value: Option<String>) -> Result<LevelFilter, ConfigError> {
    let Some(value) = value else {
        return Ok(DEFAULT_LOG_LEVEL);
    };
    let level = match value.trim().to_ascii_lowercase().as_str() {
        "trace" => LevelFilter::TRACE,
        "debug" => LevelFilter::DEBUG,
        "info" => LevelFilter::INFO,
        "warn" => LevelFilter::WARN,
        "error" => LevelFilter::ERROR,
        "off" => LevelFilter::OFF,
        _ => {
            return Err(ConfigError::InvalidValue {
                key,
                value,
                expected: LOG_LEVEL_VALUES,
            });
        }
    };
    Ok(level)
}

fn read_env_file(path: &Path) -> Result<HashMap<String, String>, ConfigError> {
    let iter = match dotenvy::from_path_iter(path) {
        Ok(iter) => iter,
        Err(error) if error.not_found() => return Ok(HashMap::new()),
        Err(error) => {
            return Err(ConfigError::EnvFile {
                path: path.to_path_buf(),
                message: error.to_string(),
            });
        }
    };

    let mut values = HashMap::new();
    for item in iter {
        match item {
            // Like dotenvy: the first declaration of a key in a file wins.
            Ok((key, value)) => {
                values.entry(key).or_insert(value);
            }
            Err(error) => {
                return Err(ConfigError::EnvFile {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                });
            }
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DATABASE_URL: &str = "postgres://test:test@localhost:5432/test";

    /// `DATABASE_URL` is required, so most tests get it for free; tests that
    /// pass their own value (or deliberately omit it) override the default.
    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let mut map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        map.entry(ENV_DATABASE_URL.to_owned())
            .or_insert_with(|| TEST_DATABASE_URL.to_owned());
        move |key| map.get(key).cloned()
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("home-orders-config-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn write(&self, name: &str, contents: &str) {
            std::fs::write(self.path.join(name), contents).expect("write env file");
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn defaults_apply_when_only_required_values_are_set() {
        let config = AppConfig::from_lookup(env_from(&[])).unwrap();

        assert_eq!(config.app_env, AppEnv::Development);
        assert_eq!(config.api_port, 3000);
        assert_eq!(config.request_log_level, LevelFilter::INFO);
        assert_eq!(config.app_log_level, LevelFilter::INFO);
        assert_eq!(config.database_url, TEST_DATABASE_URL);
        assert!(!config.cache.disabled, "the cache is enabled by default");
        assert_eq!(config.cache.host, "localhost");
        assert_eq!(config.cache.port, 6379);
        assert_eq!(config.cache.password, "");
    }

    #[test]
    fn missing_database_url_fails_fast() {
        let error = AppConfig::from_lookup(|_| None).unwrap_err();

        let message = error.to_string();
        assert!(message.contains(ENV_DATABASE_URL), "{message}");
        assert!(message.contains("missing"), "{message}");
    }

    #[test]
    fn database_url_is_trimmed_and_exposed() {
        let config = AppConfig::from_lookup(env_from(&[(
            ENV_DATABASE_URL,
            " postgres://user:pw@db.example:5432/notes ",
        )]))
        .unwrap();

        assert_eq!(
            config.database_url,
            "postgres://user:pw@db.example:5432/notes"
        );
    }

    #[test]
    fn empty_database_url_fails_fast() {
        for value in ["", "   "] {
            let error = AppConfig::from_lookup(env_from(&[(ENV_DATABASE_URL, value)])).unwrap_err();
            let message = error.to_string();
            assert!(message.contains(ENV_DATABASE_URL), "{message}");
            assert!(message.contains(DATABASE_URL_VALUES), "{message}");
        }
    }

    #[test]
    fn cache_gating_matches_fastify_exactly() {
        // Fastify parity: only unset or the exact string "false" enables the
        // cache; every other value (including "true", "1", "False", "" and
        // values with whitespace) disables it.
        let config = AppConfig::from_lookup(env_from(&[])).unwrap();
        assert!(!config.cache.disabled, "unset must enable the cache");

        let config = AppConfig::from_lookup(env_from(&[("CACHE_DISABLED", "false")])).unwrap();
        assert!(
            !config.cache.disabled,
            "the exact string false must enable the cache"
        );

        for value in ["true", "1", "False", "FALSE", " false", "false ", "0", ""] {
            let config = AppConfig::from_lookup(env_from(&[("CACHE_DISABLED", value)])).unwrap();
            assert!(
                config.cache.disabled,
                "CACHE_DISABLED={value:?} must disable the cache"
            );
        }
    }

    #[test]
    fn cache_values_override_defaults() {
        let config = AppConfig::from_lookup(env_from(&[
            ("CACHE_DISABLED", "false"),
            ("CACHE_HOST", "cache.internal"),
            ("CACHE_PORT", "6380"),
            ("CACHE_PASSWORD", "secret"),
        ]))
        .unwrap();

        assert!(!config.cache.disabled);
        assert_eq!(config.cache.host, "cache.internal");
        assert_eq!(config.cache.port, 6380);
        assert_eq!(config.cache.password, "secret");
    }

    #[test]
    fn invalid_cache_port_fails_fast() {
        for value in ["not-a-port", "70000", ""] {
            let error = AppConfig::from_lookup(env_from(&[("CACHE_PORT", value)])).unwrap_err();
            let message = error.to_string();
            assert!(message.contains("CACHE_PORT"), "{message}");
            assert!(message.contains(PORT_VALUES), "{message}");
        }
    }

    #[test]
    fn values_override_defaults() {
        let config = AppConfig::from_lookup(env_from(&[
            ("APP_ENV", "production"),
            ("API_PORT", "8080"),
            ("REQUEST_LOG_LEVEL", "debug"),
            ("APP_LOG_LEVEL", "WARN"),
        ]))
        .unwrap();

        assert_eq!(config.app_env, AppEnv::Production);
        assert!(config.app_env.is_production());
        assert_eq!(config.api_port, 8080);
        assert_eq!(config.request_log_level, LevelFilter::DEBUG);
        assert_eq!(config.app_log_level, LevelFilter::WARN);
    }

    #[test]
    fn invalid_app_env_fails_fast() {
        let error = AppConfig::from_lookup(env_from(&[("APP_ENV", "staging")])).unwrap_err();

        let message = error.to_string();
        assert!(message.contains("APP_ENV"), "{message}");
        assert!(message.contains("staging"), "{message}");
        assert!(
            message.contains("development, production or tests"),
            "{message}"
        );
    }

    #[test]
    fn invalid_port_fails_fast() {
        for value in ["not-a-port", "70000", "", "-1"] {
            let error = AppConfig::from_lookup(env_from(&[("API_PORT", value)])).unwrap_err();
            let message = error.to_string();
            assert!(message.contains("API_PORT"), "{message}");
            assert!(message.contains(PORT_VALUES), "{message}");
        }
    }

    #[test]
    fn invalid_log_level_fails_fast() {
        for (key, value) in [("REQUEST_LOG_LEVEL", "verbose"), ("APP_LOG_LEVEL", "99")] {
            let error = AppConfig::from_lookup(env_from(&[(key, value)])).unwrap_err();
            let message = error.to_string();
            assert!(message.contains(key), "{message}");
            assert!(message.contains(LOG_LEVEL_VALUES), "{message}");
        }
    }

    #[test]
    fn log_levels_accept_case_and_whitespace() {
        let config = AppConfig::from_lookup(env_from(&[
            ("REQUEST_LOG_LEVEL", " trace "),
            ("APP_LOG_LEVEL", "Off"),
        ]))
        .unwrap();

        assert_eq!(config.request_log_level, LevelFilter::TRACE);
        assert_eq!(config.app_log_level, LevelFilter::OFF);
    }

    #[test]
    fn env_files_layer_with_real_env_winning() {
        let dir = TempDir::new("layering");
        dir.write(".env", "API_PORT=4200\nAPP_LOG_LEVEL=debug\n");
        dir.write(
            ".env.development",
            "API_PORT=4100\nREQUEST_LOG_LEVEL=warn\n",
        );

        // No process env: `.env` beats `.env.development`, which beats defaults.
        let config = AppConfig::load_from(&dir.path, env_from(&[])).unwrap();
        assert_eq!(config.api_port, 4200);
        assert_eq!(config.app_log_level, LevelFilter::DEBUG);
        assert_eq!(config.request_log_level, LevelFilter::WARN);

        // Real process env beats every file.
        let config = AppConfig::load_from(&dir.path, env_from(&[("API_PORT", "4300")])).unwrap();
        assert_eq!(config.api_port, 4300);
    }

    #[test]
    fn app_env_selects_the_environment_file() {
        let dir = TempDir::new("select");
        dir.write(".env.development", "API_PORT=4100\n");
        dir.write(".env.tests", "API_PORT=4400\n");

        let config = AppConfig::load_from(&dir.path, env_from(&[])).unwrap();
        assert_eq!(
            config.api_port, 4100,
            "unset APP_ENV defaults to development"
        );

        let config = AppConfig::load_from(&dir.path, env_from(&[("APP_ENV", "tests")])).unwrap();
        assert_eq!(config.api_port, 4400);
        assert_eq!(config.app_env, AppEnv::Tests);
    }

    #[test]
    fn app_env_from_dotenv_selects_the_environment_file() {
        let dir = TempDir::new("dotenv-select");
        dir.write(".env", "APP_ENV=tests\n");
        dir.write(".env.tests", "API_PORT=4400\n");

        let config = AppConfig::load_from(&dir.path, env_from(&[])).unwrap();
        assert_eq!(config.app_env, AppEnv::Tests);
        assert_eq!(config.api_port, 4400);
    }

    #[test]
    fn rabbit_defaults_match_the_parity_contract() {
        let config = AppConfig::from_lookup(env_from(&[])).unwrap();
        let rabbit = &config.rabbit;

        assert!(
            rabbit.enabled,
            "unset RABBIT_DISABLED must enable the client"
        );
        assert_eq!(rabbit.host, "0.0.0.0");
        assert_eq!(rabbit.port, 5672, "corrected default (TS code used 5671)");
        assert_eq!(rabbit.user, "guest");
        assert_eq!(rabbit.password, "guest");
        assert_eq!(rabbit.transport, RabbitTransport::Amqp);
        assert_eq!(rabbit.address(), "amqp://guest:guest@0.0.0.0:5672");
    }

    #[test]
    fn rabbit_disabled_gate_is_an_exact_string_compare() {
        // Only the exact string "false" (or unset) enables the client; every
        // other value — including case variants and whitespace — disables it.
        for value in ["true", "1", "False", "FALSE", "", "no", " false"] {
            let config = AppConfig::from_lookup(env_from(&[("RABBIT_DISABLED", value)])).unwrap();
            assert!(
                !config.rabbit.enabled,
                "RABBIT_DISABLED={value:?} must disable the client"
            );
        }

        let config = AppConfig::from_lookup(env_from(&[("RABBIT_DISABLED", "false")])).unwrap();
        assert!(
            config.rabbit.enabled,
            "RABBIT_DISABLED=\"false\" must enable the client"
        );
    }

    #[test]
    fn rabbit_values_override_defaults() {
        let config = AppConfig::from_lookup(env_from(&[
            ("RABBIT_HOST", "localhost"),
            ("RABBIT_PORT", "5673"),
            ("RABBIT_USER", "user"),
            ("RABBIT_PASSWORD", "secret"),
            ("RABBIT_TRANSPORT", "amqps"),
        ]))
        .unwrap();
        let rabbit = &config.rabbit;

        assert_eq!(rabbit.host, "localhost");
        assert_eq!(rabbit.port, 5673);
        assert_eq!(rabbit.user, "user");
        assert_eq!(rabbit.password, "secret");
        assert_eq!(rabbit.transport, RabbitTransport::Amqps);
        assert_eq!(rabbit.address(), "amqps://user:secret@localhost:5673");
    }

    #[test]
    fn invalid_rabbit_port_and_transport_fail_fast() {
        let error = AppConfig::from_lookup(env_from(&[("RABBIT_PORT", "not-a-port")])).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("RABBIT_PORT"), "{message}");
        assert!(message.contains(PORT_VALUES), "{message}");

        let error = AppConfig::from_lookup(env_from(&[("RABBIT_TRANSPORT", "http")])).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("RABBIT_TRANSPORT"), "{message}");
        assert!(message.contains("amqp or amqps"), "{message}");
    }

    #[test]
    fn rabbit_debug_redacts_the_password() {
        let config = AppConfig::from_lookup(env_from(&[("RABBIT_PASSWORD", "hunter2")])).unwrap();

        let debug = format!("{:?}", config.rabbit);
        assert!(!debug.contains("hunter2"), "password leaked: {debug}");
        assert!(debug.contains("***"), "{debug}");
    }

    #[test]
    fn worker_defaults_match_the_parity_contract() {
        let worker = AppConfig::from_lookup(env_from(&[])).unwrap().worker;

        assert_eq!(worker.queue, "task_queue");
        assert_eq!(worker.exchange, "main");
        assert_eq!(worker.binding_key, "task.#");
        assert_eq!(
            worker.prefetch, 1,
            "WORKER_PREFETCH is new in Rust, default 1"
        );
    }

    #[test]
    fn worker_values_override_defaults() {
        let worker = AppConfig::from_lookup(env_from(&[
            ("WORKER_QUEUE", "video_queue"),
            ("WORKER_EXCHANGE", "video"),
            ("WORKER_BINDING_KEY", "video.#"),
            ("WORKER_PREFETCH", "16"),
        ]))
        .unwrap()
        .worker;

        assert_eq!(worker.queue, "video_queue");
        assert_eq!(worker.exchange, "video");
        assert_eq!(worker.binding_key, "video.#");
        assert_eq!(worker.prefetch, 16);
    }

    #[test]
    fn worker_prefetch_accepts_zero_as_unlimited() {
        let worker = AppConfig::from_lookup(env_from(&[("WORKER_PREFETCH", "0")]))
            .unwrap()
            .worker;

        assert_eq!(worker.prefetch, 0, "AMQP 0 means no prefetch limit");
    }

    #[test]
    fn invalid_worker_prefetch_fails_fast() {
        for value in ["zero", "70000", "", "-1"] {
            let error =
                AppConfig::from_lookup(env_from(&[("WORKER_PREFETCH", value)])).unwrap_err();
            let message = error.to_string();
            assert!(message.contains("WORKER_PREFETCH"), "{message}");
            assert!(message.contains(PORT_VALUES), "{message}");
        }
    }
}
