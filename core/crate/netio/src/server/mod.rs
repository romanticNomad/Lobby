pub mod auth;
pub mod txsubmit;

use crate::relayhost::handle::RelayHostHandle;
use dashmap::DashMap;
use kernel::types::{ApiKey, ClientConfig, RpcProviderRegistry};
use std::sync::Arc;
use tokio::sync::Semaphore;

// ============================================================
// middleware - app state

pub struct AppState {
    api_registry: Arc<DashMap<ApiKey, ClientConfig>>,
    rpc_registry: RpcProviderRegistry,
    pipeline_pool: Arc<Semaphore>,
    relayhost_handle: RelayHostHandle,
}

impl AppState {
    pub fn new(
        api_registry: Arc<DashMap<ApiKey, ClientConfig>>,
        rpc_registry: RpcProviderRegistry,
        pipeline_pool: Arc<Semaphore>,
        relayhost_handle: RelayHostHandle,
    ) -> Self {
        Self {
            api_registry,
            rpc_registry,
            pipeline_pool,
            relayhost_handle,
        }
    }
}

// ============================================================
