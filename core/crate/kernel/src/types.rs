use crate::traits::{EthRlpEncode, eth_rlp_append_u256};
use alloy::{
    network::AnyNetwork,
    primitives::{Address, B256, U256, bytes::Bytes},
    providers::DynProvider,
};
use core::convert::TryFrom;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::{hash::Hash, sync::Arc};
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

#[derive(Debug)]
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
//
// Designed for multi-threaded and async environments where
// providers are shared across broadcast and network execution paths.

pub type RpcProviderRegistry = Arc<DashMap<ChainId, Arc<DynProvider<AnyNetwork>>>>;

// ============================================================
// Client configuration loaded from environment variables.

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub client_id: uuid::Uuid,
    pub from_address: Address,
}

// Extension key for storing authenticated client config in request.
#[derive(Clone)]
pub struct AuthenticatedClient(pub ClientConfig);

// ============================================================
// API key type (Bearer token).

pub type ApiKey = String;

// ============================================================

pub type TxHash = B256;

// ============================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExecutionId(pub uuid::Uuid);

// ============================================================

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

// ============================================================

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

#[derive(Clone, Debug)]
pub enum LocalError {
    DatabaseError(String),
    Internal(String),
    Invariant(String),
    Rejected(String),
}

// ============================================================

#[derive(Clone, Debug)]
pub enum BroadcastOutcome {
    Submitted { txn_hash: TxHash },
    Rejected { reason: String },
    Unexpected(String),
}

// ============================================================

#[derive(Debug)]
pub enum BroadcastError {
    Internal(String),
    DatabaseError(String),
    Invariant(String),
    MissingProvider(ChainId),
}

// ============================================================
// temporary (for testing)

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

// ============================================================
