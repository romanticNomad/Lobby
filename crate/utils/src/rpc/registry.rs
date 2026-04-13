//! RPC endpoint registry construction from environment variables
//!
//! Builds dual-path topology registries (unary HTTP/2 + subscription WebSocket)
//! using Alloy-native provider construction.

use crate::rpc::{
    metadata::EndpointMetrics,
    pool::{EndpointPool, EndpointRegistry, RpcProviderStack},
};
use alloy::{
    network::Ethereum,
    providers::{Provider, ProviderBuilder, RootProvider, WsConnect},
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
    #[error(
        "Missing subscription endpoints for chain {chain_id} - every chain with unary endpoints must have corresponding subscription endpoints"
    )]
    MissingSubscriptionEndpoints { chain_id: ChainId },
    #[error("Invalid chain ID: {0}")]
    InvalidChainId(String),
    #[error("Environment variable not found: {0}")]
    EnvVarNotFound(String),
}

// ============================================================================
// Environment Variable Parsing

/// Prefix for unary (HTTP/2) endpoint environment variables
pub const UNARY_ENV_PREFIX: &str = "LOBBY_UNARY_";

/// Prefix for subscription (WebSocket) endpoint environment variables
pub const SUBSCRIPTION_ENV_PREFIX: &str = "LOBBY_SUBSCRIPTION_";

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

    /// Creates a subscription (WebSocket) provider from URL
    ///
    /// Uses `ProviderBuilder::connect_ws` for async construction.
    /// Returns a `RootProvider<Ethereum, Ws>` type-erased to `Arc<dyn Provider>`.
    pub async fn create_subscription_provider(
        url: &Url,
    ) -> Result<Arc<dyn Provider + Send + Sync>, RegistryError> {
        // Validate scheme
        if !matches!(url.scheme(), "ws" | "wss") {
            return Err(RegistryError::InvalidScheme {
                kind: "subscription",
                url: url.to_string(),
                expected: "ws:// or wss://",
            });
        }

        // Create WebSocket connection configuration
        let ws_connect = WsConnect::new(url.to_string());

        // Build provider with WebSocket transport (Alloy-native)
        let provider = ProviderBuilder::new()
            .connect_ws(ws_connect)
            .await
            .map_err(|e| RegistryError::ProviderCreation {
                url: url.to_string(),
                source: Box::new(e),
            })?;

        Ok(Arc::new(provider))
    }
}

// ============================================================================
// Registry Builders

/// Builds a complete `RpcProviderStack` from environment variables
///
/// Scans environment for `LOBBY_UNARY_*` and `LOBBY_SUBSCRIPTION_*` variables,
/// validates URL schemes, and constructs the dual-path registry.
///
/// # Environment Format
/// ```bash
/// # Unary endpoints (HTTP/2, comma-separated)
/// LOBBY_UNARY_1=https://eth-mainnet.g.alchemy.com/v2/KEY,https://eth.llamarpc.com
///
/// # Subscription endpoints (WebSocket, comma-separated)
/// LOBBY_SUBSCRIPTION_1=wss://eth-mainnet.g.alchemy.com/v2/KEY,wss://mainnet.infura.io/ws/v3/KEY
/// ```
///
/// # Validation Rules
/// - Unary URLs must use `http://` or `https://` scheme
/// - Subscription URLs must use `ws://` or `wss://` scheme
/// - Every chain with unary endpoints must have corresponding subscription endpoints
pub async fn build_registry_from_env() -> Result<RpcProviderStack, RegistryError> {
    let unary_registry = build_unary_registry_from_env().await?;
    let subscription_registry = build_subscription_registry_from_env().await?;

    // Validate: every chain with unary endpoints must have subscription endpoints
    for entry in unary_registry.iter() {
        let chain_id = *entry.key();
        if !subscription_registry.contains_key(&chain_id) {
            return Err(RegistryError::MissingSubscriptionEndpoints { chain_id });
        }
    }

    Ok(RpcProviderStack {
        unary: unary_registry,
        subscription: subscription_registry,
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

/// Builds subscription (WebSocket) endpoint registry from environment
async fn build_subscription_registry_from_env() -> Result<EndpointRegistry, RegistryError> {
    let registry: EndpointRegistry = Arc::new(DashMap::new());

    for (key, value) in std::env::vars() {
        if let Some(chain_id) = parse_chain_id_from_env(&key, SUBSCRIPTION_ENV_PREFIX) {
            let urls = parse_endpoints(&value);
            if urls.is_empty() {
                continue;
            }

            let pool = build_subscription_pool(chain_id, urls).await?;
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

        // Set tier-specific error thresholds for unary (higher tolerance for fast-path)
        // metrics.set_error_thresholds(0.15, 0.40); -----------------uncomment-later---------------------
        pool.add_endpoint(provider, metrics).await;
    }

    Ok(pool)
}

/// Builds an endpoint pool for subscription (WebSocket) providers
async fn build_subscription_pool(
    chain_id: ChainId,
    urls: Vec<String>,
) -> Result<EndpointPool, RegistryError> {
    let pool = EndpointPool::new(chain_id);

    for (idx, url_str) in urls.iter().enumerate() {
        let url = Url::parse(url_str)?;
        let provider = ProviderFactory::create_subscription_provider(&url).await?;

        let metrics = EndpointMetrics::new(format!("sub_{}_{}", chain_id, idx), url_str.clone());

        // Set tier-specific error thresholds for subscription (lower tolerance for stateful connections)
        // metrics.set_error_thresholds(0.05, 0.15); -----------------uncomment-later---------------------
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
    subscription_endpoints: Vec<(ChainId, Vec<String>)>,
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

    /// Adds subscription endpoints for a chain
    pub fn add_subscription(mut self, chain_id: ChainId, urls: Vec<String>) -> Self {
        self.subscription_endpoints.push((chain_id, urls));
        self
    }

    /// Builds the complete `RpcProviderStack`
    pub async fn build(self) -> Result<RpcProviderStack, RegistryError> {
        let unary_registry: EndpointRegistry = Arc::new(DashMap::new());
        let subscription_registry: EndpointRegistry = Arc::new(DashMap::new());

        // Build unary pools
        for (chain_id, urls) in self.unary_endpoints {
            let pool = build_unary_pool(chain_id, urls).await?;
            unary_registry.insert(chain_id, Arc::new(pool));
        }

        // Build subscription pools
        for (chain_id, urls) in self.subscription_endpoints {
            let pool = build_subscription_pool(chain_id, urls).await?;
            subscription_registry.insert(chain_id, Arc::new(pool));
        }

        // Validate: every chain with unary endpoints must have subscription endpoints
        for entry in unary_registry.iter() {
            let chain_id = *entry.key();
            if !subscription_registry.contains_key(&chain_id) {
                return Err(RegistryError::MissingSubscriptionEndpoints { chain_id });
            }
        }

        Ok(RpcProviderStack {
            unary: unary_registry,
            subscription: subscription_registry,
        })
    }
}

// ============================================================================
