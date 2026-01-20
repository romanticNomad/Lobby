use alloy_primitives::{Address, B256, Bytes, U256};
use serde::Deserialize;

// ============================================================

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

// ============================================================

pub type TxHash = B256;

// ============================================================

#[derive(Clone, Debug)]
pub struct Execution {
    pub id: ExecutionId,
    pub state: ExecutionState,
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

#[derive(Clone, Debug)]
pub struct NonceRecord {
    pub chain_id: ChainId,
    pub from: Address,
    pub nonce: TxNonce,
    pub status: NonceStatus,
    pub execution_id: ExecutionId,
}

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

pub enum Intent {
    SendTransaction(SendTransactionIntent),
}

// ============================================================

pub enum IntentResult {
    TxHash(Bytes), // Tx hash
}

// ============================================================

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
pub enum BroadcastOutcome {
    Submitted { tx_hash: TxHash },
    Rejected { reason: String }, // definitively not submitted
    Unknown,                     // may have been submitted
}

// ============================================================

#[derive(Clone, Debug)]
pub enum ExecutionError {
    InvalidIntent,
    NonceFailure,
    EncodingFailure,
    SigningFailure,
    BroadcastFailure,
    ValidationFailure,
    Internal(String),
    Rejected(String),
}

// ============================================================
