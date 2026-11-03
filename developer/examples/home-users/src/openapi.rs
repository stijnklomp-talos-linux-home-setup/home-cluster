//! OpenAPI 3.1 generated from the code (ADR-0004).
//!
//! The document is collected from the same router the app serves
//! ([`crate::routes::app_router`]), so routes and documentation cannot drift.
//! It is written on non-production startup (Fastify parity) and by
//! `just openapi`, and never hand-edited. No Swagger UI ships.

use std::path::Path;

use utoipa::openapi::{Info, License, OpenApi, Tag};

use crate::config::AppEnv;
use crate::{APP_NAME, APP_VERSION};

/// Path non-production startup and `just openapi` write to.
pub const DEFAULT_PATH: &str = "openapi.yaml";

/// The OpenAPI document for every route the app serves.
pub fn document() -> OpenApi {
    let mut document = crate::routes::app_router().into_openapi();
    // `OpenApiRouter::new` seeds `info` from utoipa-axum's *own* cargo
    // variables, so the app identity is set explicitly here.
    let mut info = Info::new(APP_NAME, APP_VERSION);
    info.description = Some(env!("CARGO_PKG_DESCRIPTION").to_owned());
    info.license = Some(License::new(env!("CARGO_PKG_LICENSE")));
    document.info = info;
    document.tags = Some(vec![Tag::new("health"), Tag::new("users")]);
    document
}

/// The generated document serialized as YAML (with a trailing newline).
pub fn to_yaml() -> std::io::Result<String> {
    let mut yaml = document().to_yaml().map_err(|error| {
        std::io::Error::other(format!("failed to serialize the OpenAPI document: {error}"))
    })?;
    if !yaml.ends_with('\n') {
        yaml.push('\n');
    }
    Ok(yaml)
}

/// Writes the generated document to `path`.
pub fn write_to(path: &Path) -> std::io::Result<()> {
    std::fs::write(path, to_yaml()?)
}

/// Startup hook: writes unless the app runs in production (Fastify parity:
/// `NODE_ENV !== "production"`); returns whether a write happened.
pub fn write_on_startup(app_env: AppEnv, path: &Path) -> std::io::Result<bool> {
    if app_env.is_production() {
        return Ok(false);
    }
    write_to(path)?;
    Ok(true)
}
