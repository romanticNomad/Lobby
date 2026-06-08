mod router;
mod state;

use crate::mockrpc::state::StateUpdateOutcome;
use async_trait::async_trait;

// ============================================================
// state contract

#[async_trait]
/// State contract for any struct that manages the `mockrcp` state system.
pub trait MockRpcState: Send + Sync + 'static {
    /// Validates and updates the stored nonce [`AtomicU64`] in the `NonceState`,
    ///
    /// ## Retruns
    /// * `StateUpdateOutcome::NonceAdvanced(u64)`, when updated successfully
    /// * `StateUpdateOutcome::NonceTooLow` in case of `rlp_nonce` < `existing_nonce`
    ///
    /// ## Note:
    /// for a controled system like benchmark, it is expected that `rlp_nonce` will not be greated than
    /// `registered nonce`.
    async fn update_nonce(&self, address: String, nonce_rlp: u64) -> StateUpdateOutcome;

    /// To reduce client overhead in the benchmarking process, the transction receipts are pre-generated,
    ///
    /// ## Returns
    /// * `StaticUpdateOutcome::TxReceipt(Arc<StaticReceipt>)`, if the receipt is found
    /// * `StaticUpdateOutcome::ReceiptNotFound`, in an unlikly case of missing receipt for the given address.
    async fn fetch_receipt(&self, address: String) -> StateUpdateOutcome;
}

// ============================================================
