use dashmap::DashMap;
use kernel::{
    traits::StateStore,
    types::{ExecutionId, PipelineStatus},
};
use redis::{AsyncCommands, aio::ConnectionManager};
use std::sync::Arc;
use uuid::Uuid;

// ============================================================
// registry

/// Shared, cheaply-cloneable registry of in-flight and recently-completed
/// pipeline statuses.
///
/// Backed by:
/// - `DashMap` (concurrent HashMap) for fast in-memory reads (O(1), no network I/O)
/// - `Redis` for distributed persistence (multi-instance support, crash-safe)
///
/// **Persistence strategy:**
/// - `set()` writes to both DashMap (immediate) and Redis (async, ~1-2ms)
/// - `get()` reads from DashMap only (no network I/O, ~1µs latency)
/// - On boot, Redis state is loaded into DashMap (full recovery)
///
/// **Redis key format:** `lobby:status:{execution_id}`
/// **TTL:** 1 hour (auto-cleanup of old entries)
#[derive(Clone)]
pub struct StatusRegistry {
    pub status_book: Arc<DashMap<ExecutionId, PipelineStatus>>,
    redis: ConnectionManager,
}

impl StatusRegistry {
    /// Create a new `StatusRegistry` with Redis persistence.
    ///
    /// ## Arguments
    /// - `redis_url`: Redis connection string (e.g., `redis://localhost:6379`)
    ///
    /// ## Errors
    /// - Returns `Err` if Redis connection fails (server unreachable, auth failure)
    /// - **Caller should panic/exit if this fails** (boot-time only)
    ///
    /// ## Recovery
    /// - Scans all `lobby:status:*` keys in Redis
    /// - Loads them into DashMap for fast local reads
    /// - Logs recovery stats (entry count, elapsed time)
    pub async fn new(redis_url: &str) -> Result<Self, redis::RedisError> {
        let start = std::time::Instant::now();

        //create redis client
        let client = redis::Client::open(redis_url)?;
        let mut redis = ConnectionManager::new(client).await?;

        tracing::debug!("redis: Client connection succesdfull");

        // load existing entries from redis to dashmap
        let status_book = Arc::new(DashMap::new());
        let mut loaded_count = 0;
        let pattern = "lobby:status:*";
        let mut cursor: u64 = 0;

        loop {
            let (new_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut redis)
                .await?;

            for key in keys {
                if let Some(uuid_str) = key.strip_prefix("lobby:status:") {
                    match Uuid::parse_str(uuid_str) {
                        Ok(uuid) => {
                            let execution_id = ExecutionId(uuid);
                            let value: Option<String> = redis.get(&key).await?;

                            if let Some(json) = value {
                                match serde_json::from_str::<PipelineStatus>(&json) {
                                    Ok(status) => {
                                        status_book.insert(execution_id, status);
                                        loaded_count += 1;
                                    }

                                    Err(e) => {
                                        tracing::error!(%execution_id, %e, "redis: status deserialize failed");
                                    }
                                }
                            }
                        }

                        Err(e) => {
                            tracing::warn!(%key, %e, "redis: invalid execution_id");
                        }
                    }
                }
            }

            cursor = new_cursor;
            if cursor == 0 {
                break;
            }
        }

        let elapsed = start.elapsed();
        tracing::info!(
            loaded_count,
            elapsed_ms = elapsed.as_millis(),
            "StatusRegistry: loaded"
        );

        Ok(Self { status_book, redis })
    }
}

// ============================================================
// StateStore implimentaion

impl StateStore for StatusRegistry {
    /// Record or overwrite the status of a pipeline.
    ///
    /// Writes to both:
    /// 1. DashMap (immediate, in-memory)
    /// 2. Redis (async, ~1-2ms, with 1-hour TTL)
    ///
    /// **Error handling:**
    /// - DashMap write always succeeds (in-memory)
    /// - Redis write failure is logged as ERROR but does NOT crash
    /// - In-memory state remains valid even if Redis write fails
    ///
    /// **TTL:** 1 hour (auto-cleanup of completed transactions)
    fn set(&self, execution_id: ExecutionId, status: PipelineStatus) {
        // update in-memory state
        self.status_book.insert(execution_id, status.clone());

        // persist to Redis (non-blocking)
        let mut redis = self.redis.clone();
        let id = execution_id;

        tokio::spawn(async move {
            let key = format!("lobby:status:{}", id);

            //serialize status to json
            let value = match serde_json::to_string(&status) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(%id, %e,"StatusRegistry: status serializing failed");
                    return;
                }
            };

            let result: Result<(), redis::RedisError> = redis.set_ex(&key, value, 3600).await;

            if let Err(e) = result {
                tracing::error!(%id, %e, "redis: write operation failed")
            }
        });
    }

    /// Retrieve the current pipeline status (reads from DashMap only, no network I/O).
    ///
    /// # Returns
    /// - `Some(status)` if execution_id exists in memory
    /// - `None` if execution_id is unknown (never submitted or TTL expired)
    fn get(&self, execution_id: &ExecutionId) -> Option<PipelineStatus> {
        self.status_book
            .get(execution_id)
            .map(|status| status.clone())
    }

    /// Explicitly delete an entry from both DashMap and Redis.
    ///
    /// **Use case:** Cleanup task to evict old `Confirmed`/`Failed` entries.
    ///
    /// **Error handling:** Redis delete failure is logged but does NOT crash.
    fn remove(&self, execution_id: &ExecutionId) {
        // remove from memory DashMap
        self.status_book.remove(execution_id);

        // remove from Redis (non - blocking)
        let mut redis = self.redis.clone();
        let id = *execution_id;

        tokio::spawn(async move {
            let key = format!("lobby:state:{}", id);
            let result: Result<(), redis::RedisError> = redis.del(&key).await;

            if let Err(e) = result {
                tracing::error!(%id, %e, "redis: status delete failed");
            }
        });
    }
}

// ============================================================
// helper functions

impl StatusRegistry {
    /// Get the approximate number of entries in the registry.
    ///
    /// **Note:** Reads from DashMap (in-memory count), not Redis.
    pub fn len(&self) -> usize {
        self.status_book.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.status_book.is_empty()
    }

    /// Get Redis connection statistics (for monitoring).
    ///
    /// **Use case:** Health checks, metrics collection.
    pub async fn redis_info(&self) -> Result<String, redis::RedisError> {
        let mut redis = self.redis.clone();
        redis::cmd("INFO")
            .arg("stats")
            .query_async(&mut redis)
            .await
    }
}

// ============================================================
