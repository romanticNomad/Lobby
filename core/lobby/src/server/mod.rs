pub mod auth;
pub mod handler;

use cortex::CortextHandle;
use dashmap::DashMap;
use kernel::types::{ApiKey, ClientConfig};
use std::sync::Arc;

// ============================================================
// middleware - app state

#[derive(Clone)]
pub struct AppState {
    api_registry: Arc<DashMap<ApiKey, ClientConfig>>,
    cortex_handler: CortextHandle,
}

impl AppState {
    pub fn new(
        api_registry: Arc<DashMap<ApiKey, ClientConfig>>,
        cortex_handler: CortextHandle,
    ) -> Self {
        Self {
            api_registry,
            cortex_handler,
        }
    }
}

// ============================================================
