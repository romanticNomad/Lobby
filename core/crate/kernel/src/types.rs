use crate::traits::{EthRlpEncode, eth_rlp_append_u256};
use alloy::{
    network::AnyNetwork,
    primitives::{Address, B256, U256, bytes::Bytes},
    providers::DynProvider,
};
use core::convert::TryFrom;
use dashmap::DashMap;
use serde::Deserialize;
use std::sync::Arc;

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

pub type TxHash = B256;

// ============================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExecutionId(pub uuid::Uuid);

// ============================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct ChainId(pub U256);

impl EthRlpEncode for ChainId {
    fn eth_rlp_append(&self, s: &mut rlp::RlpStream) {
        eth_rlp_append_u256(&self.0, s);
    }
}

// ============================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct TxNonce(pub U256);

impl TryFrom<i64> for TxNonce {
    type Error = ExecutionError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if value < 0 {
            return Err(ExecutionError::Rejected(format!("Invalid Nonce: {value}")));
        }

        Ok(TxNonce(U256::from(value as u64)))
    }
}

impl EthRlpEncode for TxNonce {
    fn eth_rlp_append(&self, s: &mut rlp::RlpStream) {
        eth_rlp_append_u256(&self.0, s);
    }
}

// ============================================================

#[derive(Clone, Debug)]
pub struct SignedTransaction {
    pub rlp: Bytes,
}

// ============================================================

#[derive(Clone, Debug)]
pub enum BroadcastOutcome {
    Submitted { txn_hash: TxHash },
    Rejected { reason: String },
    Unexpected,
}

// ============================================================

#[derive(Clone, Debug)]
pub enum ExecutionError {
    DatabaseError(String),
    Internal(String),
    Invariant(String),
    Rejected(String),
}

// ============================================================

#[derive(Debug)]
pub enum BroadcastError {
    Internal(String),
}
