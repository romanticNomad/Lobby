use primitives::types::ChainId;
use std::time::Duration;

mod calls;
mod client;
mod metrics;
mod pool;
mod registry;

// ============================================================================
// Error Types

/// Error types for RPC handling
#[derive(Debug, thiserror::Error, Clone)]
pub enum RpcError {
    #[error("Failed to acquire RPC permit within {timeout:?}")]
    PermitAcquisitionTimeout { timeout: Duration },

    #[error("Semaphore closed")]
    SemaphoreClosed,

    #[error("No RPC endpoints available for chain {chain_id}")]
    NoEndpointsAvailable { chain_id: ChainId },

    #[error("All RPC endpoints unhealthy for chain {chain_id}")]
    AllEndpointsUnhealthy { chain_id: ChainId },

    #[error("Transport error: {0}")]
    TransportError(String),

    #[error("Provider construction failed: {0}")]
    ProviderConstructionError(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
}
