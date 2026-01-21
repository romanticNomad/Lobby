use alloy_primitives::Address;
use kernel::types::{ChainId, ExecutionId, TxNonce};

// ============================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NonceStatus {
    Reserved,    // allocated to an execution, not yet broadcast
    Broadcasted, // tx sent (or assumed sent)
    Confirmed,   // included on-chain
    Dropped,     // not included, safe for replacement
}

// ============================================================

#[derive(Clone, Debug)]
pub struct NonceRecord {
    pub chain_id: ChainId,
    pub from: Address,
    pub nonce: TxNonce,
    pub status: NonceStatus,
    pub execution_id: ExecutionId,
}
