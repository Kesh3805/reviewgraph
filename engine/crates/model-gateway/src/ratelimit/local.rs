//! In-process token buckets. Used by unit tests and as the Redis fallback. Time comes from
//! `tokio::time::Instant`, so tests can run with a paused clock.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use tokio::time::Instant;

use super::{BucketSpec, BucketStore, StoreError, Take};

#[derive(Debug, Default)]
pub struct LocalBuckets {
    buckets: Mutex<HashMap<String, (f64, Instant)>>,
}

impl LocalBuckets {
    pub fn new() -> Self {
        Self::default()
    }

    fn refilled(entry: Option<&(f64, Instant)>, spec: BucketSpec, now: Instant) -> f64 {
        match entry {
            Some((tokens, ts)) => {
                let elapsed_ms = now.saturating_duration_since(*ts).as_secs_f64() * 1000.0;
                (tokens + elapsed_ms * spec.refill_per_ms).min(spec.capacity)
            }
            None => spec.capacity,
        }
    }
}

#[async_trait]
impl BucketStore for LocalBuckets {
    async fn take(&self, key: &str, spec: BucketSpec, cost: f64) -> Result<Take, StoreError> {
        let now = Instant::now();
        let mut map = self
            .buckets
            .lock()
            .map_err(|_| StoreError("poisoned".into()))?;
        let tokens = Self::refilled(map.get(key), spec, now);
        if tokens >= cost {
            map.insert(key.to_owned(), (tokens - cost, now));
            Ok(Take::Granted)
        } else {
            map.insert(key.to_owned(), (tokens, now));
            let wait_ms = ((cost - tokens) / spec.refill_per_ms).ceil();
            Ok(Take::Denied {
                wait: Duration::from_millis(wait_ms as u64),
            })
        }
    }

    async fn refund(&self, key: &str, spec: BucketSpec, amount: f64) -> Result<(), StoreError> {
        let now = Instant::now();
        let mut map = self
            .buckets
            .lock()
            .map_err(|_| StoreError("poisoned".into()))?;
        let tokens = Self::refilled(map.get(key), spec, now);
        map.insert(key.to_owned(), ((tokens + amount).min(spec.capacity), now));
        Ok(())
    }
}
