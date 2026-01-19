use alloy_primitives::{B256, U256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExecutionId(pub uuid::Uuid);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainId(pub U256);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxNonce(pub U256);
