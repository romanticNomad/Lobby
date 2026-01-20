use crate::types::{
    canonicalize::{RawTransaction, SignedTransaction},
    nonce::TxNonce,
    validate::ExecutionError,
};
use alloy_primitives::{B256, U256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExecutionId(pub uuid::Uuid);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainId(pub U256);

pub type TxHash = B256;

#[derive(Clone, Debug)]
pub enum ExecutionState {
    Registered,
    NonceReserved {
        nonce: TxNonce,
    },
    Encoded,
    Signed,
    Broadcasted {
        tx_hash: TxHash,
    },
    PendingValidation {
        tx_hash: Option<TxHash>, // None if broadcast outcome was unknown
    },
    Validated {
        tx_hash: TxHash,
        success: bool,
    },
    Failed {
        error: ExecutionError,
    },
}

#[derive(Clone, Debug)]
pub struct Execution {
    pub id: ExecutionId,
    pub state: ExecutionState,
    pub nonce: Option<TxNonce>,
    pub raw_tx: Option<RawTransaction>,
    pub signed_tx: Option<SignedTransaction>,
    pub tx_hash: Option<TxHash>,
}
