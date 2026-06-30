use primitives::types::{BroadcastError, LocalError, RelayHostError, ValidatorError};
use thiserror::Error;
use utils::rpc::LobbyRpcError;

// ============================================================
// Orchestrator (Cortex) erros

/// Every failure that the orchestrator pipeline can produce.
///
/// Variants are ordered by pipeline stage so that log aggregation tools can
/// trivially bucket failures by stage.
#[derive(Debug, Error)]
pub enum CortexError {
    /// The pipeline semaphore was exhausted and the caller timed out waiting
    /// for a permit.  The caller should back off and retry at the submission
    /// layer (i.e. return HTTP 400 / 429 to the DApp).
    #[error("pipeline semaphore timed out after {timeout_ms}ms — server is overloaded")]
    BackpressureTimeout { timeout_ms: u64 },

    /// Semaphore found closed, or the caller timed out
    #[error("EndpointPool busy or curropted: {0}")]
    EndpointPoolFailed(LobbyRpcError),

    /// netwrok congestion may have led to no healthy RPC provider being available
    #[error("No healthy RPC provider available: {0}")]
    NoHealthyRpcProvider(LobbyRpcError),

    /// error statement is self-explanatory of the purpose
    #[error("relay-host rejected or failed to record the transaction after retries: {0}")]
    RelayHost(#[from] RelayHostError),

    /// nonce reservation failed after all retries
    /// and no nonce was commited
    #[error("nonce reservation failed after retries: {0}")]
    NonceReservation(LocalError),

    /// nonce resolve (finalized / released) failed after all retries. This is non fatal
    /// but logged as an error as it indicated DB issue.
    #[error("nonce resolve failed (lease will expire): {0}")]
    NonceResolve(LocalError),

    /// nonce update in db failed. This is a fatal error, becasue it
    /// indicate mismatch of nonce in db and on-chain
    #[error("failed to sync and reserve nonce after retry: {0}")]
    NonceSync(LocalError),

    /// signing is failed after all retries. This is a fatal error
    /// nonce must be released before this error surfaces
    #[error("signing failed after retries: {0}")]
    Sign(LocalError),

    /// same fatal status as `Sign`, logged when signer fails during nonce-sync
    #[error("re-signing failed after nonce sync: {0}")]
    ReSign(LocalError),

    /// Broadcast failed after all retries. The nonce is explicitly released
    /// before this error is surfaced.
    #[error("broadcast failed after retries: {0}")]
    Broadcast(#[from] BroadcastError),

    /// The validator confirmed that the transaction was not included on-chain
    /// or faced some internal problem. The nonce is released.
    #[error("Validator failed after retries: {0}")]
    Validator(#[from] ValidatorError),

    /// internal system error
    #[error("internal orchestrator error: {0}")]
    Internal(String),
}

// ============================================================
// implimenting transient checks and staging facility for cortex errors

impl CortexError {
    /// Returns `true` for errors that are *transient* and the caller (HTTP
    /// layer) may safely surface as HTTP 429 / 503.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::BackpressureTimeout { .. })
    }

    /// Returns the pipeline stage label for structured log fields.
    pub fn stage(&self) -> &'static str {
        match self {
            Self::BackpressureTimeout { .. } => "backpressure",
            Self::EndpointPoolFailed(_) => "rpc_semaphore_permit",
            Self::NoHealthyRpcProvider(_) => "acquire_healthy_endpoint",
            Self::RelayHost(_) => "relay_host",
            Self::NonceReservation(_) => "nonce_reserve",
            Self::NonceResolve(_) => "nonce_resolve",
            Self::NonceSync(_) => "nonce_sync",
            Self::Sign(_) => "sign",
            Self::ReSign(_) => "sign_post_nonce_sync",
            Self::Broadcast(_) => "broadcast",
            Self::Validator(_) => "validator",
            Self::Internal(_) => "internal",
        }
    }
}

// ============================================================
