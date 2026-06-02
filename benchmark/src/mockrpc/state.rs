use alloy::primitives::{ChainId};
use dashmap::DashMap;
use serde::Serialize;
use std::sync::{Arc, atomic::AtomicU64};

// ============================================================

/// Primary state registry for mockrpc.
pub type ChainRegistry = DashMap<ChainId, Arc<ChainState>>;

// ============================================================

/// Lightweight EVM receipt template.
/// Uses String for hashes to avoid heavy crypto deps in the bench binary.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StaticReceipt {
    pub transaction_hash: String,
    pub status: u8, // 1 = success
    pub block_number: u64,
    pub block_hash: String,
    pub from: String,
    pub to: Option<String>,
    pub gas_used: u64,
    pub cumulative_gas_used: u64,
    pub effective_gas_price: u64,
    #[serde(rename = "type")]
    pub tx_type: u8, // 2 = EIP-1559
    pub logs: Vec<serde_json::Value>, // Empty for mock
}

// ============================================================

/// State Collection of Benchmark RPC servers.
///
/// Indexed by `ChainId` in `ChainRegistry`.
#[derive(Debug)]
pub struct ChainState {
    /// Address -> Expected Nonce (lock-free atomic advancement)
    pub nonces: DashMap<String, AtomicU64>,
    /// TxHash -> Deterministic Receipt (zero-copy reads)
    pub receipts: DashMap<String, Arc<StaticReceipt>>,
    /// Trivial block counter for `eth_blockNumber`
    pub current_block: AtomicU64,
}

// ============================================================
