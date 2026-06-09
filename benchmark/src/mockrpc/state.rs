use crate::loadgen::RECIPIENT_ADDRESS;
use crate::mockrpc::MockRpcState;
use alloy::primitives::{B256, Bloom, U64, U256};
use alloy::{
    consensus::{Eip658Value, Receipt, ReceiptEnvelope, ReceiptWithBloom},
    primitives::{Address, TxHash},
    rpc::types::TransactionReceipt,
};
use dashmap::DashMap;
use std::sync::OnceLock;
use std::sync::{Arc, atomic::AtomicU64};
use thiserror::Error;
// ============================================================

/// Constant `block_number`.
pub const BLOCK_NUMBER: u64 = 18_000_000;

// ============================================================
// State structs
pub enum NonceUpdateOutcome {
    NonceAdvanced(u64),
    NonceTooLow,
}

#[derive(Clone, Debug, Error)]
pub enum StateError {
    #[error("ChainState update failed: {0}")]
    UpdateError(String),
}
// ============================================================

/// A newtype wrapper around alloy's TransactionReceipt designed for
/// zero-overhead mock RPC responses in the benchmark harness.
#[derive(Debug)]
pub struct StaticReceipt(TransactionReceipt);

impl StaticReceipt {
    /// Generates a static, successful EIP-1559 transaction receipt.
    ///
    /// # Arguments
    /// * `tx_hash` - The transaction hash to embed in the receipt.
    pub fn gen_receipt(tx_hash: B256) -> Self {
        static BASE_RECEIPT: OnceLock<TransactionReceipt> = OnceLock::new();

        let mut receipt = BASE_RECEIPT
            .get_or_init(|| {
                // inner-consensus reciept
                let consensus_receipt = Receipt {
                    status: Eip658Value::Eip658(true), // success indicator
                    cumulative_gas_used: 21_000,
                    logs: vec![],
                };

                // ReceiptWithBloom wrapper for logs bloom filter
                let receipt_with_bloom = ReceiptWithBloom {
                    receipt: consensus_receipt,
                    logs_bloom: Bloom::ZERO,
                };

                // EIP-1559 ReceiptEnvelope (EIP-2718 typed envelope)
                let envelope = ReceiptEnvelope::Eip1559(receipt_with_bloom);

                // contrust the outer TransactionReceipt
                TransactionReceipt {
                    inner: envelope,
                    transaction_hash: B256::ZERO, // Placeholder, overwritten below
                    transaction_index: Some(1),
                    block_hash: Some(B256::ZERO),
                    block_number: Some(10_000_000),
                    gas_used: 21_000,
                    effective_gas_price: 1_000_000_000, // 1 Gwei mock price
                    blob_gas_used: None,
                    blob_gas_price: None,
                    from: Address::ZERO,
                    to: Some(Address::ZERO),
                    contract_address: None,
                }
            })
            .clone();

        // appending the transction-specific field to avoid TxHash mismatch on the client side.
        receipt.transaction_hash = tx_hash;

        Self(receipt)
    }

    /// Helper function to fetch `TxHash`
    #[inline]
    pub fn get_hash(&self) -> TxHash {
        self.0.transaction_hash
    }

    /// Provides a reference to the inner receipt for zero-copy inspection.
    #[inline]
    pub fn fetch_reciept(&self) -> &TransactionReceipt {
        &self.0
    }
}

// ============================================================

/// State Collection of Benchmark RPC servers.
///
/// Uses `String` in place of  `Address` and `TxHash` for simplicity.
#[derive(Debug)]
pub struct ChainState {
    /// Address -> Expected Nonce (lock-free atomic advancement)
    pub nonce_collection: DashMap<String, AtomicU64>,
    /// Deterministic Receipt (zero-copy reads)
    pub static_receipt: Arc<StaticReceipt>,
}

// ============================================================
// implementations for ChainState

impl ChainState {
    pub fn new(addresses: Vec<String>) -> Self {
        let nonce_collection = DashMap::new();
        for address in addresses.iter() {
            nonce_collection.insert(address.to_owned(), AtomicU64::new(0));
        }
        let static_receipt = Arc::new(StaticReceipt::gen_receipt());
        Self {
            nonce_collection,
            static_receipt,
        }
    }
}

impl MockRpcState for ChainState {
    fn update_nonce(&self, address: String, nonce_rlp: u64) -> NonceUpdateOutcome {
        // dummy reponse
        NonceUpdateOutcome::NonceTooLow
    }
    fn fetch_receipt(&self) -> Arc<StaticReceipt> {
        let receipt = self.static_receipt.clone();
        receipt
    }
}

// ============================================================
