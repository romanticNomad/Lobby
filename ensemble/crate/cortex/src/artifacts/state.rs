use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use kernel::types::{ExecutionId, PipelineStatus};
use redis::{AsyncCommands, aio::ConnectionManager};
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
    pub async fn new(redis_url: &'static str) -> Result<Self, redis::RedisError> {
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
