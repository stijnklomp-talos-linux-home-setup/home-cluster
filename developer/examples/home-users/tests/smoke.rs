//! Smoke test: the library target is linkable and exposes the app identity.

use home_users::{APP_NAME, APP_VERSION};

#[test]
fn app_identity_is_populated() {
    assert!(!APP_NAME.trim().is_empty());
    assert!(
        APP_NAME
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "APP_NAME should be a crate-style name, got {APP_NAME}"
    );
    assert!(
        is_release_version(APP_VERSION),
        "APP_VERSION should be x.y.z, got {APP_VERSION}"
    );
}

fn is_release_version(version: &str) -> bool {
    let mut parts = version.split('.');
    let three_numeric = (0..3).all(|_| {
        parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
    });
    three_numeric && parts.next().is_none()
}
