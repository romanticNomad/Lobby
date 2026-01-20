use alloy_primitives::{Address, Bytes, U256};
use crate::types::{nonce::TxNonce, state::ChainId};

pub enum Intent {
    SendTransaction(SendTransactionIntent),
}

// EIP-1159 compatible.
pub struct SendTransactionIntent {
    pub from: Address,
    pub to: Option<Address>,
    pub value: U256,
    pub data: Bytes,
    pub gas: Option<U256>,
    pub gas_price: Option<U256>,
    pub max_fee_per_gas: Option<U256>,
    pub max_priority_fee_per_gas: Option<U256>,
    pub nonce: TxNonce,
    pub chain_id: ChainId,
}

pub enum IntentResult {
    TxHash(Bytes), // Tx hash
}
