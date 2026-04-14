//! RPC endpoint registry construction from environment variables
//!
//! Builds unary topology registries (HTTP/2) using Alloy-native provider construction.

use crate::rpc::{
    metrics::EndpointMetrics,
    pool::{EndpointPool, EndpointRegistry, RpcProviderStack},
};
use alloy::{
    network::Ethereum,
    providers::{Provider, RootProvider},
};
use dashmap::DashMap;
use primitives::types::ChainId;
use std::{str::FromStr, sync::Arc};
use url::Url;

// ============================================================================
// Errors

/// Registry construction errors
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("Invalid URL scheme for {kind} endpoint: {url}. Expected: {expected}")]
    InvalidScheme {
        kind: &'static str,
        url: String,
        expected: &'static str,
    },
    #[error("Failed to parse URL: {0}")]
    UrlParse(#[from] url::ParseError),
    #[error("Failed to create provider for {url}: {source}")]
    ProviderCreation {
        url: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("Invalid chain ID: {0}")]
    InvalidChainId(String),
    #[error("Environment variable not found: {0}")]
    EnvVarNotFound(String),
}

// ============================================================================
// Environment Variable Parsing

/// Prefix for unary (HTTP/2) endpoint environment variables
pub const UNARY_ENV_PREFIX: &str = "LOBBY_UNARY_";

/// Parses chain ID from environment variable name
///
/// Format: LOBBY_UNARY_1 or LOBBY_SUBSCRIPTION_137
fn parse_chain_id_from_env(key: &str, prefix: &str) -> Option<ChainId> {
    if !key.starts_with(prefix) {
        return None;
    }

    let chain_id_str = &key[prefix.len()..];
    Some(ChainId::from_str(chain_id_str).unwrap())
}

/// Parses comma-separated endpoint URLs from environment variable value
fn parse_endpoints(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

// ============================================================================
// Provider Factory

/// Factory for creating Alloy-native providers
pub struct ProviderFactory;

impl ProviderFactory {
    /// Creates a unary (HTTP/2) provider from URL
    ///
    /// Uses `RootProvider::new_http` for synchronous construction.
    /// The provider uses hyper's HTTP/2 via ALPN negotiation.
    pub fn create_unary_provider(
        url: &Url,
    ) -> Result<Arc<dyn Provider + Send + Sync>, RegistryError> {
        // Validate scheme
        if !matches!(url.scheme(), "http" | "https") {
            return Err(RegistryError::InvalidScheme {
                kind: "unary",
                url: url.to_string(),
                expected: "http:// or https://",
            });
        }

        // Create provider using RootProvider::new_http (Alloy-native, no reqwest)
        let provider = RootProvider::<Ethereum>::new_http(url.clone());

        Ok(Arc::new(provider))
    }
}

// ============================================================================
// Registry Builders

/// Builds a complete `RpcProviderStack` from environment variables
///
/// Scans environment for `LOBBY_UNARY_*` variables,
/// validates URL schemes, and constructs the registry.
///
/// # Environment Format
/// ```bash
/// # Unary endpoints (HTTP/2, comma-separated)
/// LOBBY_UNARY_1=https://eth-mainnet.g.alchemy.com/v2/KEY,https://eth.llamarpc.com
/// ```
///
/// # Validation Rules
/// - Unary URLs must use `http://` or `https://` scheme
pub async fn build_registry_from_env() -> Result<RpcProviderStack, RegistryError> {
    let unary_registry = build_unary_registry_from_env().await?;

    Ok(RpcProviderStack {
        unary: unary_registry,
    })
}

/// Builds unary (HTTP/2) endpoint registry from environment
async fn build_unary_registry_from_env() -> Result<EndpointRegistry, RegistryError> {
    let registry: EndpointRegistry = Arc::new(DashMap::new());

    for (key, value) in std::env::vars() {
        if let Some(chain_id) = parse_chain_id_from_env(&key, UNARY_ENV_PREFIX) {
            let urls = parse_endpoints(&value);
            if urls.is_empty() {
                continue;
            }

            let pool = build_unary_pool(chain_id, urls).await?;
            registry.insert(chain_id, Arc::new(pool));
        }
    }

    Ok(registry)
}

/// Builds an endpoint pool for unary (HTTP/2) providers
async fn build_unary_pool(
    chain_id: ChainId,
    urls: Vec<String>,
) -> Result<EndpointPool, RegistryError> {
    let pool = EndpointPool::new(chain_id);

    for (idx, url_str) in urls.iter().enumerate() {
        let url = Url::parse(url_str)?;
        let provider = ProviderFactory::create_unary_provider(&url)?;

        let metrics = EndpointMetrics::new(format!("unary_{}_{}", chain_id, idx), url_str.clone());

        // Set error thresholds (default for unary calls: 0.15, 0.40)
        metrics.set_error_thresholds(0.15, 0.40);
        pool.add_endpoint(provider, metrics).await;
    }

    Ok(pool)
}

// ============================================================================
// Manual Registry Construction

/// Builder for programmatic registry construction
#[derive(Debug, Default)]
pub struct RegistryBuilder {
    unary_endpoints: Vec<(ChainId, Vec<String>)>,
}

impl RegistryBuilder {
    /// Creates a new registry builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds unary endpoints for a chain
    pub fn add_unary(mut self, chain_id: ChainId, urls: Vec<String>) -> Self {
        self.unary_endpoints.push((chain_id, urls));
        self
    }

    /// Builds the complete `RpcProviderStack`
    pub async fn build(self) -> Result<RpcProviderStack, RegistryError> {
        let unary_registry: EndpointRegistry = Arc::new(DashMap::new());

        // Build unary pools
        for (chain_id, urls) in self.unary_endpoints {
            let pool = build_unary_pool(chain_id, urls).await?;
            unary_registry.insert(chain_id, Arc::new(pool));
        }

        Ok(RpcProviderStack {
            unary: unary_registry,
        })
    }
}

// ============================================================================
