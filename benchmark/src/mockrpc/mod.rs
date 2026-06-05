use crate::mockrpc::state::ChainState;
use dashmap::DashMap;
use std::sync::Arc;

mod router;
mod state;

/// Primary app registry for mockrpc.
pub type ChainRegistry = DashMap<u64, Arc<ChainState>>;
