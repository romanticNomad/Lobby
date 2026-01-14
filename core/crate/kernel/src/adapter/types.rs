use alloy_primitives::{Address, Bytes, U256};

pub enum Intent {
    SendTransaction(SendTransactionIntent),
    ChainId,
}

pub struct SendTransactionIntent {
    pub from: Address,
    pub to: Option<Address>,
    pub value: U256,
    pub data: Bytes,
    pub gas: Option<U256>,
    pub gas_price: Option<U256>,
    pub nonce: Option<U256>,
    pub chain_id: Option<U256>,
}

pub enum IntentResult {
    Bytes(Bytes), // Tx hash
    U256(U256),   // chain_id
}

#[derive(Debug)]
pub enum IntentError {
    Rejected(String),
    Invalid(String),
    Internal(String),
}
