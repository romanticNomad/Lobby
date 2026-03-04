pub mod auth;
pub mod handler;

use cortex::{CortextHandle, artifacts::state::StatusRegistry};
use kernel::types::ApiRegistry;

// ============================================================
// app state

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) api_registry: ApiRegistry,       // authentication
    pub(crate) cortex_handler: CortextHandle,   // POST handler
    pub(crate) status_registry: StatusRegistry, // Get handler
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
// defining substate modifications for auth_middleware and get_status, handlers.

impl axum::extract::FromRef<AppState> for StatusRegistry {
    fn from_ref(state: &AppState) -> Self {
        state.status_registry.clone()
    }
}

impl axum::extract::FromRef<AppState> for ApiRegistry {
    fn from_ref(state: &AppState) -> Self {
        state.api_registry.clone()
    }
}

// ============================================================
