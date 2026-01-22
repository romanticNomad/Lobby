use alloy_primitives::{Address, B256, Bytes, U256};
use serde::Deserialize;

// ============================================================

// EIP-1159 compatible.
#[derive(Debug)]
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

// ============================================================

pub type TxHash = B256;

// ============================================================

#[derive(Debug)]
pub struct Execution {
    pub id: ExecutionId,
    pub state: ExecutionState,
    pub payload: Intent,
    pub nonce: Option<TxNonce>,
    pub raw_tx: Option<RawTransaction>,
    pub signed_tx: Option<SignedTransaction>,
    pub tx_hash: Option<TxHash>,
}

// ============================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExecutionId(pub uuid::Uuid);

// ============================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct ChainId(pub U256);

// ============================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct TxNonce(pub U256);

// ============================================================

#[derive(Clone, Debug)]
pub struct RawTransaction {
    pub rlp: Bytes,
}

// ============================================================

#[derive(Clone, Debug)]
pub struct SignedTransaction {
    pub rlp: Bytes,
}

// ================================================

#[derive(Debug)]
pub enum Intent {
    SendTransaction(SendTransactionIntent),
}

// ============================================================

pub enum IntentResult {
    TxHash(TxHash), // Tx hash
}

// ============================================================

#[derive(Clone, Debug)]
pub enum ExecutionState {
    Registered,
    NonceReserved {
        nonce: TxNonce,
    },
    Canonicalized,
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

// ============================================================

#[derive(Clone, Debug)]
pub enum BroadcastOutcome {
    Submitted { tx_hash: TxHash },
    Rejected { reason: String },
    Unknown,
}

// ============================================================

#[derive(Clone, Debug)]
pub enum ExecutionError {
    BroadcastFailure,
    Internal(String),
    Rejected(String),
}

// ============================================================
