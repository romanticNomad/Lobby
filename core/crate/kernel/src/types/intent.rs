use alloy_primitives::{Address, Bytes, U256};

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
    pub nonce: Option<U256>,
    pub chain_id: Option<U256>,
}

pub enum IntentResult {
    TxHash(Bytes), // Tx hash
}

#[derive(Debug)]
pub enum IntentError {
    Rejected(String),
    Invalid(String),
    Internal(String),
}
