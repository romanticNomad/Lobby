use crate::traits::{EthRlpEncode, eth_rlp_append_u256};
use alloy::{
    network::AnyNetwork,
    primitives::{Address, B256, U256, bytes::Bytes},
    providers::DynProvider,
};
use core::convert::TryFrom;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::{fmt, hash::Hash, sync::Arc};
use thiserror::Error;

// ============================================================
// EIP-1193 eth_sendTransaction request parameters.

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
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
// Concurrent registry of per-chain RPC providers.

pub type RpcProviderRegistry = Arc<DashMap<ChainId, Arc<DynProvider<AnyNetwork>>>>;

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
// API key type (Bearer token).

pub type ApiKey = String;

// ============================================================
// JSON-RPC wrappers for Lobby (eip1193)

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: Vec<Eip1193SendTransactionParams>,
    pub id: serde_json::Value,
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct TxnAcceptedResult {
    pub execution_id: ExecutionId,
    pub status: String,
}

// ============================================================
// TxHash lobby wrapper for B256

pub type TxHash = B256;

// ============================================================
// idempotency key for lobby operations

#[derive(Clone, Copy, Debug, Serialize, Hash, PartialEq, Eq)]
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

impl fmt::Display for ChainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================
// TxNonce wrapper for lobby and appropriate implimentations

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
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

// ============================================================

#[derive(Clone, Debug)]
pub struct SignedTransaction {
    pub rlp: Bytes,
}

// ============================================================
// Possible outcome response (even if failed)

#[derive(Clone, Debug)]
pub enum BroadcastOutcome {
    Submitted { txn_hash: TxHash },
    Rejected { reason: String },
    Unexpected(String),
}

// ============================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationOutcome {
    /// Transaction has >= 1 on-chain confirmations.
    Included,
    /// Transaction hash is unknown to the node — likely a chain reorg or the
    /// tx was evicted from the mempool.  Caller must release the nonce.
    NotIncluded,
}

// ============================================================
// errors for different execution levels

#[derive(Debug, Error)]
pub enum RelayHostError {
    #[error("Duplicate execution_id: {0}")]
    DuplicateExecutionId(uuid::Uuid),

    #[error("Invalid transaction: {0}")]
    ValidationFailed(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("Actor has shut down")]
    ActorShutdown,

    #[error("From address mismatch: expected {expected}, got {actual}")]
    FromAddressMismatch { expected: String, actual: String },
}

#[derive(Clone, Debug, Error)]
pub enum LocalError {
    #[error("Db error: {0}")]
    DatabaseError(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Internal error: {0}")]
    Invariant(String),
    #[error("Rejected: {0}")]
    Rejected(String),
}

#[derive(Debug, Error)]
pub enum BroadcastError {
    #[error("Internal: {0}")]
    Internal(String),
    #[error("DB error: {0}")]
    DatabaseError(String),
    #[error("Invariant error: {0}")]
    Invariant(String),
    #[error("Provider error: {:?}", chain_id)]
    MissingProvider { chain_id: ChainId },
}

#[derive(Debug, Error)]
pub enum ValidatorError {
    #[error(
        "validation timed out waiting for tx {tx_hash} on chain {:?}",
        chain_id
    )]
    Timeout { chain_id: ChainId, tx_hash: TxHash },

    #[error("rpc error while polling for tx {tx_hash}: {message}")]
    Rpc { tx_hash: TxHash, message: String },

    #[error("transaction not included after max confirmations: {tx_hash}")]
    NotIncluded { tx_hash: TxHash },
}

// ============================================================
