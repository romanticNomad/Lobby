use std::time::Duration;
use thiserror::Error;

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
            base_delay: Duration::from_millis(50), // Was 100 - faster retry for high TPS
            max_delay: Duration::from_millis(150), // Was 200 - reduced max delay
        }
    }
}

// ============================================================
// # orchestrator (cortex) config

/// Top-level config for the orchestrator, loaded entirely from environment
/// variables so that production tuning never requires a recompile.
#[derive(Debug, Clone)]
pub struct CortexConfig {
    pub nonce_shards: usize,
    pub sign_shards: usize,
    pub broadcast_shards: usize,
    pub validator_shards: usize,
    pub actor_buffer: usize,
    pub pipeline_concurrency: usize,
    pub pipeline_semaphore_timeout: Duration,
    pub retry: RetryConfig,
}

impl CortexConfig {
    /// Load from environment variables, falling back to sensible defaults for
    /// any variable that is absent.
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            nonce_shards: parse_env("NONCE_SHARDS", 17)?,
            sign_shards: parse_env("SIGN_SHARDS", 17)?,
            broadcast_shards: parse_env("BROADCAST_SHARDS", 17)?,
            validator_shards: parse_env("VALIDATOR_SHARDS", 17)?,
            actor_buffer: parse_env("ACTOR_BUFFER_SIZE", 256)?,
            pipeline_concurrency: parse_env("PIPELINE_CONCURRENCY", 200)?,
            pipeline_semaphore_timeout: Duration::from_millis(parse_env(
                "PIPELINE_SEMAPHORE_TIMEOUT_MS",
                5_000u64,
            )?),
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

/// Parse an environment variable into a given type
fn parse_env<T>(key: &'static str, default: T) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match std::env::var(key) {
        Ok(raw) => raw.parse::<T>().map_err(|e| ConfigError::Parse {
            key,
            source: Box::new(e),
        }),
        Err(_) => Ok(default),
    }
}

// ============================================================
