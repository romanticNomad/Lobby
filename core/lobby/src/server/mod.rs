pub mod auth;
pub mod handler;

use cortex::{CortextHandle, state::StatusRegistry};
use dashmap::DashMap;
use kernel::types::{ApiKey, ClientConfig};
use std::sync::Arc;

// ============================================================
// middleware - app state

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) api_registry: Arc<DashMap<ApiKey, ClientConfig>>,
    pub(crate) cortex_handler: CortextHandle,
    pub(crate) status_registry: Arc<StatusRegistry>,
}

impl AppState {
    pub fn new(
        api_registry: Arc<DashMap<ApiKey, ClientConfig>>,
        cortex_handler: CortextHandle,
        status_registry: Arc<StatusRegistry>,
    ) -> Self {
        Self {
            api_registry,
            cortex_handler,
            status_registry,
        }
    }
}

// ============================================================

// allowing status_registry to be a sub_state used for get_transaction_status
impl axum::extract::FromRef<AppState> for Arc<StatusRegistry> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.status_registry)
    }
}

// ============================================================
