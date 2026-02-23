use std::time::Duration;
use thiserror::Error;
use utils::directory::parse_env;

// ============================================================
//  # Retry

/// Retry policy applied uniformly to every pipeline stage.
///
/// Strategy: **full-jitter exponential backoff**
/// `for 'nth' attempt`
/// * window = min(max_delay, base_delay * 2^n)
/// * delay = random(0, window)
///
/// Rationale: Under load, many pipelines may fail at the same instant
/// (e.g. RPC node briefly overloaded or DB hiccup).  Full jitter scatters
/// retries across the whole window, preventing a thundering-herd re-spike.
/// With max_attempts = 2 the worst-case added latency stays well under a
/// second while still absorbing transient faults.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

// setting default config for retires across pipelines
impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 2,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(200),
        }
    }
}

// ============================================================
// # orchestrator (cortex) config

/// Top-level config for the orchestrator, loaded entirely from environment
/// variables so that production tuning never requires a recompile.
#[derive(Debug, Clone)]
pub struct CortexConfig {
    pub nonce_shard: usize,
    pub sign_shard: usize,
    pub broadcast_shard: usize,
    pub actor_buffer: usize,
    pub pipeline_concurrency: usize,
    pub pipeline_semaphore_timeout: Duration,
    pub retry: RetryConfig,
}

impl CortexConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            nonce_shard: parse_env("NONCE_SHARDS", 17).map_err(|e| ConfigError::Parse {
                key: "NONCE_SHARDS",
                source: Box::new(e),
            })?,
            sign_shard: parse_env("SIGN_SHARDS", 17).map_err(|e| ConfigError::Parse {
                key: "SIGN_SHARDS",
                source: Box::new(e),
            })?,
            broadcast_shard: parse_env("BROADCAST_SHARDS", 17).map_err(|e| ConfigError::Parse {
                key: "BROADCAST_SHARDS",
                source: Box::new(e),
            })?,
            actor_buffer: parse_env("BROADCAST_SHARDS", 64).map_err(|e| ConfigError::Parse {
                key: "BROADCAST_SHARDS",
                source: Box::new(e),
            })?,
            pipeline_concurrency: parse_env("PIPELINE_CONCURRENCY", 17).map_err(|e| {
                ConfigError::Parse {
                    key: "PIPELINE_CONCURRENCY",
                    source: Box::new(e),
                }
            })?,
            pipeline_semaphore_timeout: Duration::from_millis(
                parse_env("PIPELINE_SEMAPHORE_TIMEOUT_MS", 5_000u64).map_err(|e| {
                    ConfigError::Parse {
                        key: "PIPELINE_SEMAPHORE_TIMEOUT_MS",
                        source: Box::new(e),
                    }
                })?,
            ),
            retry: RetryConfig::default(),
        })
    }
}

// ============================================================
// ConfigError

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("environment variable '{key}' is set but cannot be parsed: {source}")]
    Parse {
        key: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

// ============================================================
