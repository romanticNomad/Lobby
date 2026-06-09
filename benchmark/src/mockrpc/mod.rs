mod router;
mod state;

use crate::mockrpc::state::{NonceUpdateOutcome, StaticReceipt};
use alloy::primitives::TxHash;
use alloy::rpc::types::TransactionReceipt;
use async_trait::async_trait;
use std::sync::Arc;
// ============================================================
// state contract

/// State contract for any struct that manages the `mockrcp` state system.
pub trait MockRpcState: Send + Sync {
    /// Validates and updates the stored nonce [`AtomicU64`] in the `NonceState`,
    ///
    /// ## Retruns
    /// * `StateUpdateOutcome::NonceAdvanced(u64)`, when updated successfully
    /// * `StateUpdateOutcome::NonceTooLow` in case of `rlp_nonce` < `existing_nonce`
    ///
    /// ## Note:
    /// for a controled system like benchmark, it is expected that `rlp_nonce` will not be greated than
    /// `registered nonce`.
    fn update_nonce(&self, address: String, nonce_rlp: u64) -> NonceUpdateOutcome;

    /// To reduce client overhead in the benchmarking process, the transction receipts are pre-generated,
    ///
    /// ## Returns
    /// * `TransactionReceipt`, if the receipt is found
    /// * panics if the `tx_hash` is not already registere.
    fn fetch_receipt(&self, tx_hash: TxHash) -> TransactionReceipt;
}

// ============================================================
