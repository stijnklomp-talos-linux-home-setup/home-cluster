//! Values shared by the two Pact crates (`tests/pact/consumer.rs` and
//! `tests/pact/provider.rs`); each crate `#[path]`-includes this file because
//! integration-test crates cannot share modules through the library.

/// Id the user contract uses for the single-user interaction. The provider
/// state inserts exactly this row, and teardown deletes every fixture id at or
/// above [`PACT_USER_ID`] — never hand-seeded rows.
#[allow(dead_code)]
pub const PACT_USER_ID: i64 = 900_001;

/// Id of the second fixture user (the list interaction).
#[allow(dead_code)]
pub const PACT_SECOND_USER_ID: i64 = 900_002;
