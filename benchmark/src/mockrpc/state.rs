use std::sync::{Arc, atomic::AtomicU64};

use alloy::primitives::{Address, B256, ChainId, TxHash};
use dashmap::DashMap;

/// Primary state registry for mockrpc.
pub type ChainRegistry = DashMap<ChainId, Arc<ChainState>>;

/// State Collection of Benchmark RPC servers.
///
/// Indexed by `ChainId` in `ChainRegistry`.
#[derive(Debug)]
pub struct ChainState {
    nonce: DashMap<Address, AtomicU64>,
    tx_reciept: DashMap<TxHash, B256>,
}
