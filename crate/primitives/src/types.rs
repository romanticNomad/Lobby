use crate::traits::{EthRlpEncode, eth_rlp_append_u256};
use alloy::primitives::{Address, B256, U256, bytes::Bytes};
use core::convert::TryFrom;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::{fmt, hash::Hash, sync::Arc};
use thiserror::Error;

// ============================================================
// EIP-1193 eth_sendTransaction request parameters.

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Eip1193SendTransactionParams {
    pub from: Address,
    pub to: Option<Address>,
    #[serde(default)]
    pub gas: Option<String>, // Hex string
    #[serde(default)]
    pub max_fee_per_gas: Option<String>, // Hex string
    #[serde(default)]
    pub max_priority_fee_per_gas: Option<String>, // Hex string
    #[serde(default)]
    pub value: Option<String>, // Hex string
    #[serde(default)]
    pub data: Option<String>, // Hex string
    pub chain_id: String, // Hex string
    #[serde(default)]
    pub access_list: Option<Vec<AccessListItem>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessListItem {
    pub address: Address,
    pub storage_keys: Vec<String>, // Hex strings
}

// ============================================================
// Struct for eip-1159 rlp encoding.

#[derive(Debug, Clone)]
pub struct Eip1559Transaction {
    pub chain_id: ChainId,
    pub nonce: TxNonce,
    pub max_priority_fee_per_gas: U256,
    pub max_fee_per_gas: U256,
    pub gas_limit: U256,
    pub to: Option<Address>, // None = contract creation
    pub value: U256,
    pub data: Bytes,
    pub access_list: Vec<(Address, Vec<U256>)>,
}

// ============================================================
// client configuration loaded from environment variables and
// extension key for storing authenticated client config in request.

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub client_id: uuid::Uuid,
    pub from_address: Address,
}

#[derive(Clone)]
pub struct AuthenticatedClient(pub ClientConfig);

// ============================================================

/// A type alias for concurrent HashMap (DashMap) that maps
/// api keys to corresponding client configurations (captured from envirment variables),
/// wrapped in `Arc` so it can be cheaply cloned.
pub type ApiRegistry = Arc<DashMap<ApiToken, ClientConfig>>;

pub type ApiToken = String;

// ============================================================
// JSON-RPC wrappers for Lobby handlers

#[derive(Debug, Deserialize, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: Vec<Eip1193SendTransactionParams>,
    pub id: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcSuccessResponse {
    pub jsonrpc: String,
    pub result: TxnAcceptedResult,
    pub id: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcErrorResponse {
    pub jsonrpc: String,
    pub error: JsonRpcError,
    pub id: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TxnAcceptedResult {
    pub execution_id: ExecutionId,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonStatusResponse {
    pub execution_id: String,
    #[serde(flatten)]
    pub status: PipelineStatus,
}

// ============================================================
// TxHash lobby wrapper for B256

pub type TxHash = B256;

// ============================================================
// idempotency key for lobby operations

#[derive(Clone, Copy, Debug, Serialize, Hash, PartialEq, Eq, Deserialize)]
pub struct ExecutionId(pub uuid::Uuid);

impl fmt::Display for ExecutionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================
// ChainID wrapper for lobby and appropriate implimentations

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Hash)]
pub struct ChainId(pub U256);

impl EthRlpEncode for ChainId {
    fn eth_rlp_append(&self, s: &mut rlp::RlpStream) {
        eth_rlp_append_u256(&self.0, s);
    }
}

impl TryFrom<i64> for ChainId {
    type Error = LocalError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if value < 0 {
            return Err(LocalError::Rejected(format!("Invalid chain id: {value}")));
        }

        Ok(ChainId(U256::from(value as u64)))
    }
}

impl std::str::FromStr for ChainId {
    type Err = LocalError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value: i64 = s
            .parse()
            .map_err(|e| LocalError::Rejected(format!("Invalid chain id string `{s}`: {e}")))?;

        ChainId::try_from(value)
    }
}

impl fmt::Display for ChainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================
// TxNonce wrapper for lobby and appropriate implimentations

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct TxNonce(pub U256);

impl EthRlpEncode for TxNonce {
    fn eth_rlp_append(&self, s: &mut rlp::RlpStream) {
        eth_rlp_append_u256(&self.0, s);
    }
}

impl TryFrom<i64> for TxNonce {
    type Error = LocalError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if value < 0 {
            return Err(LocalError::Rejected(format!("Invalid Nonce: {value}")));
        }

        Ok(TxNonce(U256::from(value as u64)))
    }
}

impl fmt::Display for TxNonce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================

#[derive(Clone, Debug)]
pub struct SignedTransaction {
    rlp: Bytes,
    with_nonce: TxNonce,
}

impl SignedTransaction {
    pub fn new(rlp: Bytes, with_nonce: TxNonce) -> Self {
        Self { rlp, with_nonce }
    }
    
    #[inline]
    pub fn rlp(&self) -> &Bytes {
        &self.rlp
    }

    #[inline]
    pub fn with_nonce(&self) -> TxNonce {
        self.with_nonce
    }
}

// ============================================================

#[derive(Clone, Debug)]
pub struct BroadcastOutcome {
    pub txn_hash: TxHash,
}

// =========================================================
// nonce state type mapping for sqlx <-> Postgres

#[derive(Debug, Clone, Copy, PartialEq, sqlx::Type)]
#[sqlx(type_name = "nonce.nonce_state", rename_all = "lowercase")]
pub enum NonceState {
    Reserved,
    Finalized,
    Released,
    Consumed,
}

// =========================================================
// sign state type mapping for sqlx <-> Postgres

#[derive(Debug, Clone, Copy, PartialEq, sqlx::Type)]
#[sqlx(type_name = "sign.sign_requests", rename_all = "lowercase")]
pub enum SignState {
    Reserved,
    Signed,
    Failed,
}

// ============================================================
// tracking pipeline status

/// Coarse-grained lifecycle states that the orchestrator pipeline advances
/// through for each `ExecutionId`.
///
/// The status is written optimistically (no locking beyond DashMap's per-shard
/// locks) — readers may briefly see a stale value, but the transitions are
/// monotonic (states only advance forward or to Failed).
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum PipelineStatus {
    /// pipeline semaphore permit aquired
    PermitAquired,
    /// request has been accepted and persisted by RelayHost
    Accepted,
    /// nonce successfully reserved, awaiting signer
    NonceReserved,
    /// transaction signed, awaiting broadcaster
    Signed,
    /// transaction broadcasted, awaiting on-chain confirmation
    Broadcasted {
        #[serde(rename = "tx_hash")]
        tx_hash: String,
        messege: String,
    },
    /// Validator confirmed >=1 block confirmation
    ConfirmedOnChain {
        #[serde(rename = "tx_hash")]
        tx_hash: String,
    },
    /// Pipeline failed at the given stage; the nonce has been released where
    /// applicable.
    Failed { stage: String, reason: String },
    /// Syncing nonce on db with the on-chain nonce
    /// retrieved using the given rpc_endpoint
    NonceMismatchDetected {
        nonce_on_chain: TxNonce,
        attempted_nonce: TxNonce,
    },
    /// Validator timed out without confirmation (due to high nonce),
    /// such situation might be created due to nonce gaps
    ValidatorTimedOut { message: String },
}

// ============================================================
// outcomes returned by validator.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidatorOutcome {
    /// confirmation recieved from RPC
    Included {
        block_number: u64,
        confirmations: u64,
    },
    /// RPC returns status 0, or error responce
    NotIncluded,
    /// validator timed out
    /// scanner_bot will poll rpc for confirmation
    Timeout,
}

// ============================================================
// errors for different execution levels

#[derive(Debug, Error)]
pub enum RelayHostError {
    /// Transaction was rejected by RelayHost validation (client-facing failure).
    #[error("Validation Failed {0}")]
    ValidationFailed(String),
    /// Database failure while recording RelayHost state.
    #[error("Database Error: {0}")]
    DatabaseError(#[from] sqlx::Error),
    /// Catch-all for unexpected internal failures (bugs / integration issues).
    #[error("Internal: {0}")]
    Internal(String),
    /// Authenticated `from` address did not match the transaction `from`.
    #[error("From Address Mismatch: expected {expected}, got {actual}")]
    FromAddressMismatch { expected: String, actual: String },
}

/// Errors for local (non-RPC) pipeline stages such as Nonce reservation and Sign.
#[derive(Debug, Error)]
pub enum LocalError {
    /// Database query/execute failure represented as a string for easy surfacing.
    #[error("Db error: {0}")]
    DatabaseError(String),
    /// Catch-all for unexpected internal failures (including actor shutdown).
    #[error("Internal error: {0}")]
    Internal(String),
    /// An internal consistency invariant was violated (usually a bug or bad state).
    #[error("Internal error: {0}")]
    Invariant(String),
    /// Request was rejected due to invalid inputs or policy.
    #[error("Rejected: {0}")]
    Rejected(String),
}

#[derive(Debug, Error)]
pub enum BroadcastError {
    /// Catch-all for unexpected internal failures (including actor shutdown).
    #[error("Internal: {0}")]
    Internal(String),
    /// Database failure while recording broadcast state.
    #[error("DB error: {0}")]
    DatabaseError(String),
    /// An internal consistency invariant was violated (usually a bug or bad state).
    #[error("Invariant error: {0}")]
    Invariant(String),
    /// No RPC provider was configured for the given chain.
    #[error("Provider error: {:?}", chain_id)]
    MissingProvider { chain_id: ChainId },
    /// Provider deterministically rejected the transaction (nonce/funds/etc).
    #[error("Tx request rejected by provider: {}", reason)]
    Rejected { reason: String },
    /// Provider failed in a non-deterministic / unexpected way.
    #[error("Unexpected error: {}", message)]
    Unexpected { message: String },
    /// Broadcast failed due to 'nonce too low'
    #[error("nonce too low -> fetching correct nonce from rpc node")]
    NonceTooLow {
        nonce_on_chain: TxNonce,
        attempted_nonce: TxNonce,
    },
    /// RPC related failiures
    #[error("Rpc Failiure: {0}")]
    RpcError(String),
}

#[derive(Debug, Error)]
pub enum ValidatorError {
    /// RPC node returned an error while polling for the transaction receipt.
    #[error("rpc error while polling for tx {tx_hash}: {message}")]
    Rpc { tx_hash: TxHash, message: String },
    /// Transaction was mined but with status=0 (execution reverted).
    #[error("transaction {tx_hash:#x} reverted on-chain (status=0)")]
    Reverted { tx_hash: TxHash },
    /// Database error while recording validation state.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// fallback internal code error
    #[error("internal error: {0}")]
    Internal(String),
    /// RPC related failiures
    #[error("Rpc Failiure: {0}")]
    RpcError(String),
}

// ============================================================
