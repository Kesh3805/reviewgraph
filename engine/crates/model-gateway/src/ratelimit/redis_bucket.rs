//! Redis token buckets (feature `redis`). One atomic Lua script per take; Redis `TIME` is the
//! shared clock. Keys are `rg:rl:{provider}:{model}:{dim}` and carry no tenant or source data.

use std::time::Duration;

use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::Script;

use super::{BucketSpec, BucketStore, StoreError, Take};

const TAKE_LUA: &str = include_str!("token_bucket.lua");
const REFUND_LUA: &str = include_str!("refund.lua");

pub struct RedisBuckets {
    conn: ConnectionManager,
    take: Script,
    refund: Script,
}

impl std::fmt::Debug for RedisBuckets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisBuckets").finish_non_exhaustive()
    }
}

fn err(e: redis::RedisError) -> StoreError {
    StoreError(e.to_string())
}

impl RedisBuckets {
    /// Connects (with automatic reconnection). `url` comes from `REDIS_URL`.
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        let client = redis::Client::open(url).map_err(err)?;
        let conn = ConnectionManager::new(client).await.map_err(err)?;
        Ok(Self {
            conn,
            take: Script::new(TAKE_LUA),
            refund: Script::new(REFUND_LUA),
        })
    }
}

#[async_trait]
impl BucketStore for RedisBuckets {
    async fn take(&self, key: &str, spec: BucketSpec, cost: f64) -> Result<Take, StoreError> {
        let mut conn = self.conn.clone();
        let (granted, wait_ms): (i64, i64) = self
            .take
            .key(key)
            .arg(spec.capacity)
            .arg(spec.refill_per_ms)
            .arg(cost)
            .invoke_async(&mut conn)
            .await
            .map_err(err)?;
        if granted == 1 {
            Ok(Take::Granted)
        } else {
            Ok(Take::Denied {
                wait: Duration::from_millis(u64::try_from(wait_ms).unwrap_or(0)),
            })
        }
    }

    async fn refund(&self, key: &str, spec: BucketSpec, amount: f64) -> Result<(), StoreError> {
        let mut conn = self.conn.clone();
        let _: i64 = self
            .refund
            .key(key)
            .arg(spec.capacity)
            .arg(spec.refill_per_ms)
            .arg(amount)
            .invoke_async(&mut conn)
            .await
            .map_err(err)?;
        Ok(())
    }
}
