//! Real Valkey: the cache client connects, stores and reads back.

use home_users::infrastructure::cache::CacheClient;

use crate::{cache_config, unique_label};

#[tokio::test]
async fn cache_round_trips_against_real_valkey() {
    let mut cache = CacheClient::connect(&cache_config())
        .await
        .expect("the acceptance cache must be reachable");
    assert!(cache.is_initialized(), "the client must have connected");

    let key = format!("acceptance:{}", unique_label("cache"));
    cache.set(&key, "value-1").await.expect("SET");
    assert_eq!(
        cache.get(&key).await.expect("GET"),
        Some("value-1".to_owned())
    );

    assert_eq!(cache.del(&key).await.expect("DEL"), 1);
    assert_eq!(cache.get(&key).await.expect("GET after DEL"), None);

    cache.close();
}
