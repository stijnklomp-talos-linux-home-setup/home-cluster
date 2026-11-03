//! Writes the generated OpenAPI document (`just openapi`, ADR-0004).
//!
//! An argument overrides the output path; the default is the committed
//! `openapi.yaml` at the repository root.

use std::path::Path;
use std::process::ExitCode;

use home_orders::openapi;

fn main() -> ExitCode {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| openapi::DEFAULT_PATH.to_owned());

    match openapi::write_to(Path::new(&path)) {
        Ok(()) => {
            println!("wrote {path}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("failed to write {path}: {error}");
            ExitCode::FAILURE
        }
    }
}
