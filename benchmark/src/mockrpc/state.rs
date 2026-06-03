use alloy::primitives::ChainId;
use dashmap::DashMap;
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::{Arc, atomic::AtomicU64};
// ============================================================

/// Primary state registry for mockrpc.
pub type ChainRegistry = DashMap<ChainId, Arc<ChainState>>;

// ============================================================

/// Lightweight EVM receipt template.
/// Uses String for hashes for simplicity.
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
/// Uses `String` in place of  `Address` and `TxHash` for simplicity.
#[derive(Debug)]
pub struct ChainState {
    /// Address -> Expected Nonce (lock-free atomic advancement)
    pub nonce_collection: DashMap<String, AtomicU64>,
    /// TxHash -> Deterministic Receipt (zero-copy reads)
    pub receipt_collection: DashMap<String, Arc<StaticReceipt>>,
    /// Trivial block counter for `eth_blockNumber`
    pub current_block: AtomicU64,
}

// ============================================================
// implementations for ChainState

impl ChainState {
    pub fn new(addresses: Vec<String>) -> Self {
        let nonce_collection = DashMap::new();
        for address in addresses {
            nonce_collection.insert(address, AtomicU64::new(0));
        }

        let receipt_collection = DashMap::new();
        let current_block = AtomicU64::new(18_000_000); // A dummy block number

        Self {
            nonce_collection,
            receipt_collection,
            current_block,
        }
    }

    /// Called after successful nonce validation in `eth_sendRawTransaction`
    pub fn store_receipt(
        &self,
        tx_hash: String,
        from: String,
        to: Option<String>,
    ) -> Arc<StaticReceipt> {
        let block = self.current_block.fetch_add(1, Ordering::Relaxed);
        let receipt = Arc::new(StaticReceipt {
            transaction_hash: tx_hash.clone(),
            status: 1,
            block_number: block,
            block_hash: format!("0x{:0>64}", "mockblockhash"),
            from,
            to,
            gas_used: 21_000,
            cumulative_gas_used: 21_000,
            effective_gas_price: 1_000_000_000,
            tx_type: 2,
            logs: vec![],
        });

        self.receipt_collection.insert(tx_hash, receipt.clone());
        receipt
    }
}

// ============================================================
