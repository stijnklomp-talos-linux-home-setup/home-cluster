//! OpenAPI generation behavior, ported from the Fastify template's server
//! tests: the document is generated from the code (ADR-0004), written on
//! non-production startup and by `just openapi`, and never hand-edited.

use std::path::PathBuf;

use home_orders::config::AppEnv;
use home_orders::openapi;

/// Per-test temp directory (the process id keeps parallel test binaries apart).
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("home-orders-openapi-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn file(&self) -> PathBuf {
        self.path.join("openapi.yaml")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn write_on_startup_writes_the_document_on_non_production() {
    for app_env in [AppEnv::Development, AppEnv::Tests] {
        let dir = TempDir::new(app_env.as_str());
        let path = dir.file();

        let wrote = openapi::write_on_startup(app_env, &path).unwrap();

        assert!(wrote, "{app_env:?} must write");
        let document = std::fs::read_to_string(&path).unwrap();
        assert!(document.starts_with("openapi: 3.1.0"), "{document:.120}");
        assert!(document.contains("/v1/orders"), "orders path missing");
        assert!(document.contains("Problem"), "Problem schema missing");
    }
}

#[test]
fn write_on_startup_does_not_write_on_production() {
    let dir = TempDir::new("production");
    let path = dir.file();

    let wrote = openapi::write_on_startup(AppEnv::Production, &path).unwrap();

    assert!(!wrote, "production must not write");
    assert!(!path.exists(), "production must not create the file");
}

#[test]
fn document_covers_health_and_orders_with_generated_schemas() {
    let document = openapi::document();
    let json = document.to_json().unwrap();

    for path in ["/healthz", "/readyz", "/v1/orders", "/v1/orders/{id}"] {
        assert!(json.contains(path), "{path} missing from the document");
    }
    for schema in ["Order", "OrdersResponse", "Problem"] {
        assert!(json.contains(schema), "{schema} schema missing");
    }
}

#[test]
fn committed_openapi_yaml_is_the_generated_document() {
    // Integration tests run from the package root, next to `openapi.yaml`.
    let committed = std::fs::read_to_string("openapi.yaml")
        .expect("openapi.yaml is committed; run `just openapi` to refresh it");
    let generated = openapi::to_yaml().unwrap();

    assert_eq!(
        committed, generated,
        "openapi.yaml is stale; run `just openapi` (never hand-edit it)"
    );
}
