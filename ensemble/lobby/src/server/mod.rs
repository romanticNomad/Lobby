pub mod auth;
pub mod handler;

use cortex::{CortextHandle, artifacts::state::StatusRegistry};
use kernel::types::ApiRegistry;

// ============================================================
// app state

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) api_registry: ApiRegistry,
    pub(crate) cortex_handler: CortextHandle,
    pub(crate) status_registry: StatusRegistry,
}

impl AppState {
    pub fn new(
        api_registry: ApiRegistry,
        cortex_handler: CortextHandle,
        status_registry: StatusRegistry,
    ) -> Self {
        Self {
            api_registry,
            cortex_handler,
            status_registry,
        }
    }
}

// ============================================================

// allowing status_registry to be a sub_state used for get_transaction_status handler
impl axum::extract::FromRef<AppState> for StatusRegistry {
    fn from_ref(state: &AppState) -> Self {
        state.status_registry.clone()
    }
}

// ============================================================
