//! Exercises the cache client (`SET`/`GET`/`DEL`) against the configured Valkey.
//!
//! ```sh
//! CACHE_DISABLED=false CACHE_HOST=localhost CACHE_PASSWORD=admin123 \
//!     cargo run --example cache
//! ```

use std::process::ExitCode;

use home_orders::config::AppConfig;
use home_orders::infrastructure::cache::{CacheClient, CacheError};

#[tokio::main]
async fn main() -> ExitCode {
    let config = match AppConfig::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("failed to load configuration: {error}");
            return ExitCode::FAILURE;
        }
    };

    let mut cache = match CacheClient::connect(&config.cache).await {
        Ok(cache) => cache,
        Err(error) => {
            eprintln!("cache connect failed: {error}");
            return ExitCode::FAILURE;
        }
    };

    if !cache.is_initialized() {
        println!("cache is disabled; run with CACHE_DISABLED=false to exercise the client");
        return ExitCode::SUCCESS;
    }

    let result = demo(&cache).await;
    cache.close();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cache command failed: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn demo(cache: &CacheClient) -> Result<(), CacheError> {
    let key = "rust-template:cache-example";
    cache.set(key, "hello").await?;
    let value = cache.get(key).await?;
    let deleted = cache.del(key).await?;
    println!("SET {key} hello -> ok");
    println!("GET {key} -> {value:?}");
    println!("DEL {key} -> {deleted}");
    Ok(())
}
