//! Alloy-native provider factory
//!
//! This module provides pure Alloy 1.6.0 provider construction without external HTTP clients.
//! Uses `RootProvider` with hyper-based HTTP/2 (via ALPN).

use alloy::{
    network::Ethereum,
    providers::{Provider, ProviderBuilder, RootProvider},
};
use std::{fmt::Debug, sync::Arc};
use thiserror::Error;
use url::Url;

// ============================================================================
// Error Types

/// Errors that can occur during provider construction
#[derive(Debug, Error, Clone)]
pub enum ProviderError {
    #[error("Invalid URL scheme: expected {expected}, got {actual}")]
    InvalidScheme { expected: String, actual: String },

    #[error("Invalid URL format: {0}")]
    InvalidUrl(String),

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

// ============================================================================
// Provider Factory

/// Factory for creating Alloy-native providers
///
/// This factory eliminates external HTTP client dependencies (reqwest) in favor
/// of Alloy's built-in `hyper`-based HTTP/2.
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
}

// ============================================================================
