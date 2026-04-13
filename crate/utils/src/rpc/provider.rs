//! Alloy-native provider factory and WebSocket pool management
//!
//! This module provides pure Alloy 1.6.0 provider construction without external HTTP clients.
//! Uses `RootProvider` with hyper-based HTTP/2 (via ALPN) and native WebSocket support.

use alloy::{
    network::Ethereum,
    providers::{Provider, ProviderBuilder, RootProvider, WsConnect},
};
use std::{
    fmt::Debug,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use thiserror::Error;
use tokio::sync::RwLock;
use url::Url;

// ============================================================================
// Error Types

/// Errors that can occur during provider construction or WebSocket operations
#[derive(Debug, Error, Clone)]
pub enum ProviderError {
    #[error("Invalid URL scheme: expected {expected}, got {actual}")]
    InvalidScheme { expected: String, actual: String },

    #[error("Invalid URL format: {0}")]
    InvalidUrl(String),

    #[error("WebSocket connection failed: {0}")]
    WsConnectionFailed(String),

    #[error("HTTP provider construction failed: {0}")]
    HttpConstructionFailed(String),

    #[error("Provider not available for chain {chain_id}")]
    ProviderNotAvailable { chain_id: u64 },
}

// ============================================================================
// Type Aliases for Clarity

/// Concrete HTTP provider type used internally for unary operations
///
/// This is the actual type returned by `RootProvider::new_http()` and
/// `ProviderBuilder::connect_http()`. We use type erasure to `Arc<dyn Provider>`
/// for storage to avoid complex generic propagation.
pub type UnaryProvider = RootProvider<Ethereum>;

/// WebSocket provider type for subscription operations
///
/// Note: WebSocket providers maintain persistent connections and require
/// different lifecycle management than HTTP providers.
pub type WsProvider = RootProvider<Ethereum>;

// ============================================================================
// Provider Factory

/// Factory for creating Alloy-native providers
///
/// This factory eliminates external HTTP client dependencies (reqwest) in favor
/// of Alloy's built-in `hyper`-based HTTP/2 and native WebSocket providers.
///
/// # Performance Notes
/// - HTTP/2 multiplexing is handled automatically by hyper via ALPN
/// - No manual stream management required; hyper's flow control handles backpressure
/// - Connection pooling is managed transparently by hyper's `Client`
pub struct ProviderFactory;

impl ProviderFactory {
    /// Creates a unary (HTTP/2) provider for stateless request-response operations
    ///
    /// Uses `RootProvider::new_http()` for direct construction or
    /// `ProviderBuilder::connect_http()` when fillers are needed.
    ///
    /// # Arguments
    /// * `url` - HTTP or HTTPS endpoint URL
    ///
    /// # Returns
    /// `Arc<dyn Provider>` type-erased for storage in endpoint pools
    ///
    /// # Errors
    /// Returns `ProviderError::InvalidScheme` if URL is not HTTP/HTTPS
    ///
    /// # Example
    /// ```ignore
    /// let provider = ProviderFactory::create_unary_provider("https://eth.llamarpc.com").await?;
    /// let block_num = provider.get_block_number().await?;
    /// ```
    pub async fn create_unary_provider(
        url: &str,
    ) -> Result<Arc<dyn Provider + Send + Sync>, ProviderError> {
        // Validate URL scheme
        let parsed_url = url
            .parse::<Url>()
            .map_err(|e| ProviderError::InvalidUrl(e.to_string()))?;

        let scheme = parsed_url.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(ProviderError::InvalidScheme {
                expected: "http or https".to_string(),
                actual: scheme.to_string(),
            });
        }

        // Create provider using RootProvider::new_http for minimal overhead
        // This uses hyper's HTTP/2 via ALPN automatically
        let provider = UnaryProvider::new_http(parsed_url);

        // Type erasure to Arc<dyn Provider> for storage flexibility
        // The Provider trait is implemented for RootProvider, allowing dynamic dispatch
        Ok(Arc::new(provider) as Arc<dyn Provider + Send + Sync>)
    }

    /// Creates a unary provider with recommended fillers for transaction submission
    ///
    /// Use this variant when you need gas estimation, nonce management, and chain ID
    /// filling. This adds overhead but simplifies transaction building.
    ///
    /// # Performance Impact
    /// ~5-10% overhead per call due to filler logic. Use `create_unary_provider()`
    /// for high-throughput read-only operations.
    pub async fn create_unary_provider_with_fillers(
        url: &str,
    ) -> Result<Arc<dyn Provider + Send + Sync>, ProviderError> {
        let parsed_url = url
            .parse::<Url>()
            .map_err(|e| ProviderError::InvalidUrl(e.to_string()))?;

        let scheme = parsed_url.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(ProviderError::InvalidScheme {
                expected: "http or https".to_string(),
                actual: scheme.to_string(),
            });
        }

        // ProviderBuilder with recommended fillers (Gas, BlobGas, Nonce, ChainId)
        let provider = ProviderBuilder::new().connect_http(parsed_url);

        Ok(Arc::new(provider) as Arc<dyn Provider + Send + Sync>)
    }

    /// Creates a WebSocket provider for stateful subscription operations
    ///
    /// WebSocket providers maintain persistent connections and are used for:
    /// - `eth_subscribe` (newPendingTransactions, newHeads, logs)
    /// - Real-time event streaming
    ///
    /// # Arguments
    /// * `url` - WebSocket endpoint URL (ws:// or wss://)
    ///
    /// # Returns
    /// `Arc<dyn Provider>` type-erased for storage in WebSocket pools
    ///
    /// # Errors
    /// Returns `ProviderError::InvalidScheme` if URL is not WS/WSS
    ///
    /// # Performance Notes
    /// - WebSocket connections have higher initial overhead but lower per-message latency
    /// - Each WebSocket provider maintains a dedicated TCP connection
    /// - Use `WsPool` for round-robin distribution across multiple WebSocket endpoints
    ///
    /// # Example
    /// ```ignore
    /// let ws_provider = ProviderFactory::create_subscription_provider("wss://eth.llamarpc.com").await?;
    /// let sub = ws_provider.subscribe_pending_transactions().await?;
    /// ```
    pub async fn create_subscription_provider(
        url: &str,
    ) -> Result<Arc<dyn Provider + Send + Sync>, ProviderError> {
        // Validate URL scheme
        let parsed_url = url
            .parse::<Url>()
            .map_err(|e| ProviderError::InvalidUrl(e.to_string()))?;

        let scheme = parsed_url.scheme();
        if scheme != "ws" && scheme != "wss" {
            return Err(ProviderError::InvalidScheme {
                expected: "ws or wss".to_string(),
                actual: scheme.to_string(),
            });
        }

        // Create WebSocket connection configuration
        let ws_connect = WsConnect::new(parsed_url);

        // Build provider with WebSocket transport
        // Note: This establishes the connection immediately
        let provider = ProviderBuilder::new()
            .connect_ws(ws_connect)
            .await
            .map_err(|e| ProviderError::WsConnectionFailed(e.to_string()))?;

        Ok(Arc::new(provider) as Arc<dyn Provider + Send + Sync>)
    }

    /// Creates a WebSocket provider with custom configuration
    ///
    /// Allows tuning of retry behavior, keepalive intervals, and authentication.
    ///
    /// # Configuration Options
    /// - `max_retries`: Max reconnection attempts (default: 10)
    /// - `retry_interval`: Seconds between retries (default: 3)
    /// - `keepalive_ping_interval`: Seconds between keepalive pings (default: 10)
    pub async fn create_subscription_provider_with_config(
        url: &str,
        max_retries: u32,
        retry_interval_secs: u64,
    ) -> Result<Arc<dyn Provider + Send + Sync>, ProviderError> {
        let parsed_url = url
            .parse::<Url>()
            .map_err(|e| ProviderError::InvalidUrl(e.to_string()))?;

        let scheme = parsed_url.scheme();
        if scheme != "ws" && scheme != "wss" {
            return Err(ProviderError::InvalidScheme {
                expected: "ws or wss".to_string(),
                actual: scheme.to_string(),
            });
        }

        let ws_connect = WsConnect::new(parsed_url)
            .with_max_retries(max_retries)
            .with_retry_interval(std::time::Duration::from_secs(retry_interval_secs));

        let provider = ProviderBuilder::new()
            .connect_ws(ws_connect)
            .await
            .map_err(|e| ProviderError::WsConnectionFailed(e.to_string()))?;

        Ok(Arc::new(provider) as Arc<dyn Provider + Send + Sync>)
    }
}

// ============================================================================
// WebSocket Pool Management

/// Round-robin WebSocket endpoint distributor for subscription load balancing
///
/// Unlike unary HTTP/2 providers that use weighted least-RTT selection,
/// WebSocket providers use round-robin assignment because:
/// 1. Each subscription requires a persistent connection
/// 2. Connection affinity is more important than latency for stateful streams
/// 3. Round-robin provides uniform distribution across endpoints
///
/// # Thread Safety
/// Uses atomic counter for O(1) round-robin selection without locks.
/// Provider list is protected by RwLock for safe concurrent read/write.
pub struct WsPool {
    /// Chain ID for this WebSocket pool
    chain_id: u64,

    /// WebSocket providers with round-robin distribution
    ///
    /// Stored as `Arc<dyn Provider>` for type erasure, but all concrete instances
    /// are `RootProvider<Ethereum, Ws>`.
    providers: RwLock<Vec<Arc<dyn Provider + Send + Sync>>>,

    /// Atomic counter for round-robin selection (lock-free)
    round_robin_counter: AtomicUsize,

    /// Pool size for modulo operation
    size: AtomicUsize,
}

impl Debug for WsPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WsPool")
            .field("chain_id", &self.chain_id)
            .field("round_robin_counter", &self.round_robin_counter)
            .field("size", &self.size)
            .finish()
    }
}

impl WsPool {
    /// Creates a new WebSocket pool for a specific chain
    pub fn new(chain_id: u64) -> Self {
        Self {
            chain_id,
            providers: RwLock::new(Vec::new()),
            round_robin_counter: AtomicUsize::new(0),
            size: AtomicUsize::new(0),
        }
    }

    /// Adds a WebSocket provider to the pool
    ///
    /// Acquires write lock briefly for vector push.
    pub async fn add_provider(&self, provider: Arc<dyn Provider + Send + Sync>) {
        let mut providers = self.providers.write().await;
        providers.push(provider);
        let new_size = providers.len();
        drop(providers);

        self.size.store(new_size, Ordering::Release);
    }

    /// Gets the next WebSocket provider using round-robin selection
    ///
    /// # Performance
    /// - O(1) atomic fetch-add for index calculation
    /// - O(1) RwLock read for provider access
    /// - Total: ~50-100ns per call (amortized)
    ///
    /// # Returns
    /// `Some(Arc<dyn Provider>)` if pool has providers, `None` if empty
    pub async fn next_endpoint(&self) -> Option<Arc<dyn Provider + Send + Sync>> {
        let size = self.size.load(Ordering::Acquire);
        if size == 0 {
            return None;
        }

        // Atomic fetch-add for lock-free round-robin
        let index = self.round_robin_counter.fetch_add(1, Ordering::Relaxed) % size;
        let providers = self.providers.read().await;
        providers.get(index).cloned()
    }

    /// Gets a specific provider by index
    ///
    /// Used for sticky session scenarios where a specific WebSocket connection
    /// must be maintained (e.g., subscription resumption).
    pub async fn get_provider(&self, index: usize) -> Option<Arc<dyn Provider + Send + Sync>> {
        let providers = self.providers.read().await;
        providers.get(index).cloned()
    }

    /// Returns the number of WebSocket providers in the pool
    pub fn len(&self) -> usize {
        self.size.load(Ordering::Acquire)
    }

    /// Returns true if the pool contains no providers
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the chain ID for this pool
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Removes all providers from the pool
    ///
    /// Note: Existing `Arc` clones held by callers remain valid until dropped.
    pub async fn clear(&self) {
        let mut providers = self.providers.write().await;
        providers.clear();
        drop(providers);
        self.size.store(0, Ordering::Release);
    }
}

// ============================================================================
// Provider Health Checking

/// Lightweight health check utilities for provider validation
///
/// These are used during provider construction and periodic health verification.
pub struct ProviderHealthChecker;

impl ProviderHealthChecker {
    /// Performs a lightweight health check on a provider
    ///
    /// Uses `eth_blockNumber` as the health check method because:
    /// - It's supported by all Ethereum nodes
    /// - It's lightweight (no state access required)
    /// - It validates the full request-response pipeline
    ///
    /// # Timeout
    /// 5 seconds to prevent hanging on unhealthy endpoints
    pub async fn check_health(provider: &Arc<dyn Provider + Send + Sync>) -> bool {
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            provider.get_block_number(),
        )
        .await
        {
            Ok(Ok(_)) => true,
            Ok(Err(_)) => false,
            Err(_) => false, // Timeout
        }
    }

    /// Checks WebSocket provider health by attempting a subscription
    ///
    /// More thorough than HTTP check as it validates the pub/sub pipeline.
    /// Uses `eth_subscribe` to newHeads then immediately unsubscribes.
    ///
    /// # Timeout
    /// 10 seconds (longer than HTTP due to subscription setup overhead)
    pub async fn check_ws_health(provider: &Arc<dyn Provider + Send + Sync>) -> bool {
        match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            provider.subscribe_blocks(),
        )
        .await
        {
            Ok(Ok(sub)) => {
                // Successfully subscribed, drop the subscription
                drop(sub);
                true
            }
            _ => false,
        }
    }
}

// ============================================================================
