use crate::loadgen::RECIPIENT_ADDRESS;
use crate::mockrpc::MockRpcState;
use alloy::{
    consensus::{Eip658Value, Receipt, ReceiptEnvelope, ReceiptWithBloom},
    primitives::{Address, B256, Bloom, TxHash},
    rpc::types::TransactionReceipt,
};
use dashmap::DashMap;
use serde_json::value::RawValue;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};
use thiserror::Error;

// ============================================================
//constants

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

/// Receipt holder struct designed for minimum heap-allocation
/// and fast serialization
///
/// ## Field
/// *`raw_json`: Pre-serialized JSON payload wrapped in a RawValue.
/// Cloning this Arc is an O(1) atomic reference count increment.
#[derive(Debug)]
pub struct StaticReceipt {
    raw_json: Arc<Box<RawValue>>,
}

impl StaticReceipt {
    /// Generates a static, successful EIP-1559 transaction receipt.
    ///
    /// # Arguments
    /// * `tx_hash` - The transaction hash to embed in the receipt.
    ///
    /// `COLD PATH` operation: Serializes the receipt exactly once at creation time.
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
                    block_number: Some(BLOCK_NUMBER),
                    gas_used: 21_000,
                    effective_gas_price: 1_000_000_000, // 1 Gwei mock price
                    blob_gas_used: None,
                    blob_gas_price: None,
                    from: RECIPIENT_ADDRESS.parse::<Address>().unwrap(),
                    to: Some(Address::ZERO),
                    contract_address: None,
                }
            })
            .clone();

        // appending the transction-specific field to avoid TxHash mismatch on the client side.
        receipt.transaction_hash = tx_hash;

        // Serialize directly to String (Exactly 1 heap allocation)
        let json_string =
            serde_json::to_string(&receipt).expect("TransactionReceipt must be serializable");

        // Convert to RawValue.
        // This consumes the String and stores it internally as a Box<str>.
        let raw_json = RawValue::from_string(json_string).expect("Generated JSON must be valid");

        Self {
            raw_json: Arc::new(raw_json),
        }
    }

    /// Provides a reference to the inner receipt Bytes for zero-copy inspection.
    #[inline]
    pub fn payload(&self) -> &Arc<Box<RawValue>> {
        &self.raw_json
    }
}

// ============================================================

/// State Collection of Benchmark RPC servers.
///
/// Uses `String` in place of  `Address` and `TxHash` for simplicity.
#[derive(Debug)]
pub struct ChainState {
    /// Address -> Expected Nonce (lock-free atomic advancement)
    pub nonce_collection: DashMap<Address, AtomicU64>,
    /// TxHash -> StaticReceipt map (appends on `BASE_RECEIPT`)
    pub receipt_collection: DashMap<TxHash, Arc<StaticReceipt>>,
}

// ============================================================
// implementations for ChainState

impl ChainState {
    pub fn new(addresses: Vec<Address>) -> Self {
        let nonce_collection = DashMap::new();
        for address in addresses.iter() {
            nonce_collection.insert(address.to_owned(), AtomicU64::new(0));
        }
        let receipt_collection = DashMap::new();
        Self {
            nonce_collection,
            receipt_collection,
        }
    }

    /// appends the `TxHash` to the `BASE_RECEIPT` and inserts the
    /// `(TxHash, StaticReceipt)` pair to the `receipt_collection`Dashmap.
    pub fn update_receipt(&self, tx_hash: TxHash) {
        let receipt_collection = StaticReceipt::gen_receipt(tx_hash);
        self.receipt_collection
            .insert(tx_hash, Arc::new(receipt_collection));
    }
}

impl MockRpcState for ChainState {
    /// Validates and updates the stored nonce [`AtomicU64`] in the `NonceState`.
    ///
    /// ## Returns
    /// * `NonceUpdateOutcome::NonceAdvanced(u64)`, when updated successfully.
    /// * `NonceUpdateOutcome::NonceTooLow` in case of `rlp_nonce` < `existing_nonce`.
    ///
    /// ## Note on Gap Tolerance:
    /// In a strictly controlled benchmark, `rlp_nonce` is expected to exactly match
    /// the registered nonce. However, if `rlp_nonce > current` (e.g., due to a dropped
    /// packet or out-of-order execution in the harness), we advance the state to
    /// `rlp_nonce + 1`. This mimics an EVM mempool accepting future nonces and prevents
    /// the benchmark from deadlocking on a permanent gap.
    fn update_nonce(&self, address: Address, nonce_rlp: u64) -> NonceUpdateOutcome {
        // Note: The `entry` API performs a single lookup. It holds the DashMap shard lock
        // only for the duration of the CAS loop below (which is strictly bounded
        // and executes in nanoseconds), preventing race conditions on initialization.
        let entry = self
            .nonce_collection
            .entry(address)
            .or_insert_with(|| AtomicU64::new(0));
        let atomic_nonce = entry.value();

        // Lock-free CAS (Compare-And-Swap) loop for atomic advancement
        let mut current_nonce = atomic_nonce.load(Ordering::Relaxed);

        loop {
            // Strictly reject nonces that have already been consumed.
            if nonce_rlp < current_nonce {
                return NonceUpdateOutcome::NonceTooLow;
            }

            // Target state: The next expected nonce is always `nonce_rlp + 1`.
            let next_expected = current_nonce + 1;

            // Attempt atomic update.
            // We use `compare_exchange_weak` which is allowed to fail spuriously.
            match atomic_nonce.compare_exchange_weak(
                current_nonce,
                next_expected,
                Ordering::Relaxed, // success ordering
                Ordering::Relaxed, // failiure ordering
            ) {
                Ok(_) => return NonceUpdateOutcome::NonceAdvanced(current_nonce),
                Err(actual) => {
                    // CAS failed because another thread updated the nonce concurrently
                    // (or a spurious failure occurred). Update `current` and retry.
                    current_nonce = actual;
                }
            }
        }
    }

    /// HOT PATH: Fetches the pre-serialized raw json str.
    ///
    /// ## Benchmark Safety
    /// Returns `Option<Arc<Box<RawValue>>>`. The `Arc::clone()` inside the map
    /// is an atomic increment, taking < 5 nanoseconds.
    fn fetch_receipt(&self, tx_hash: &TxHash) -> Option<Arc<Box<RawValue>>> {
        self.receipt_collection
            .get(tx_hash)
            .map(|entry| entry.value().payload().clone())
    }
}

// ============================================================
