use crate::types::state::{ChainId, ExecutionId};
use alloy_primitives::{Address, U256};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct TxNonce(pub U256);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NonceStatus {
    Reserved,    // allocated to an execution, not yet broadcast
    Broadcasted, // tx sent (or assumed sent)
    Confirmed,   // included on-chain
    Dropped,     // not included, safe for replacement
}

#[derive(Clone, Debug)]
pub struct NonceRecord {
    pub chain_id: ChainId,
    pub from: Address,
    pub nonce: TxNonce,

    pub status: NonceStatus,
    pub execution_id: ExecutionId,
}
