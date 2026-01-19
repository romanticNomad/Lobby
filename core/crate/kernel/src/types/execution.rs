use crate::types::{
    id::ExecutionId,
    tx_artifacts::{RawTransaction, SignedTransaction, TxHash, TxNonce},
};

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

    PendingFinality {
        tx_hash: Option<TxHash>, // None if broadcast outcome was unknown
    },

    Finalized {
        tx_hash: TxHash,
        success: bool,
    },

    Failed {
        error: ExecutionError,
    },
}

#[derive(Clone, Debug)]
pub enum ExecutionError {
    InvalidIntent,
    NonceFailure,
    EncodingFailure,
    SigningFailure,
    BroadcastFailure,
    FinalityFailure,
    Internal(String),
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
