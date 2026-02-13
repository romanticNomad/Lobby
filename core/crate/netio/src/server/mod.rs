pub mod auth;
pub mod post;

use crate::relayhost::handle::RelayHostHandle;
use axum::{Router, middleware, routing::post};
use dashmap::DashMap;
use kernel::types::{ApiKey, ClientConfig};
use std::sync::Arc;

// ============================================================
// middleware - app state

#[derive(Clone)]
pub struct AppState {
    api_registry: Arc<DashMap<ApiKey, ClientConfig>>,
    relayhost_handle: RelayHostHandle,
}

impl AppState {
    pub fn new(
        api_registry: Arc<DashMap<ApiKey, ClientConfig>>,
        relayhost_handle: RelayHostHandle,
    ) -> Self {
        Self {
            api_registry,
            relayhost_handle,
        }
    }
}

// ============================================================
// build axum app router

pub fn build_app(state: AppState) -> Router {

}

// ============================================================
