mod router;
mod state;

use crate::mockrpc::state::NonceUpdateOutcome;
use alloy::primitives::{Address, TxHash};
use serde_json::value::RawValue;
use std::sync::Arc;

// ============================================================
// state contract

/// State contract for any struct that manages the `mockrcp` state system.
pub trait MockRpcState: Send + Sync {
    /// Validates and updates the stored nonce [`AtomicU64`] in the `NonceState`,
    ///
    /// ## Retruns
    /// * `NonceUpdateOutcome::NonceAdvanced(u64)`, when updated successfully
    /// * `NonceUpdateOutcome::NonceTooLow` in case of `rlp_nonce` < `existing_nonce`
    ///
    /// ## Note:
    /// for a controled system like benchmark, it is expected that `rlp_nonce` will not be greated than
    /// `registered nonce`.
    fn update_nonce(&self, address: Address, nonce_rlp: u64) -> NonceUpdateOutcome;

    /// To reduce client overhead in the benchmarking process, the transction receipts are pre-generated,
    ///
    /// ## Returns
    /// * `Some(Arc<Box<RawValue>>)`: return raw_value, with zero heap cloning.
    /// * `None`: when no receipt is yet registered.
    fn fetch_receipt(&self, tx_hash: &TxHash) -> Option<Arc<Box<RawValue>>>;
}

// ============================================================
// re-exports

pub use router::RpcAppState;
