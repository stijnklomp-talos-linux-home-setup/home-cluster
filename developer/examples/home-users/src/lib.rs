//! Library root for the service app.

pub mod amqp;
pub mod config;
pub mod consumer;
pub mod controllers;
pub mod database;
#[cfg(feature = "graphql")]
pub mod graphql;
pub mod health;
pub mod infrastructure;
pub mod logging;
pub mod models;
pub mod openapi;
pub mod problem;
pub mod repositories;
pub mod routes;
pub mod server;
pub mod services;
pub mod shutdown;
pub mod validation;

/// Package name of the service app, as declared in `Cargo.toml`.
pub const APP_NAME: &str = env!("CARGO_PKG_NAME");

/// Package version of the service app, as declared in `Cargo.toml`.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
