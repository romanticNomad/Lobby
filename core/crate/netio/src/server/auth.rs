use std::sync::Arc;

use dashmap::DashMap;
use kernel::types::{ApiKey, ClientConfig};

// ============================================================
// middleware - auth state

#[derive(Clone)]
pub struct AuthState {
    api_registry: Arc<DashMap<ApiKey, ClientConfig>>
}

impl AuthState {
    pub fn new (api_registry: Arc<DashMap<ApiKey, ClientConfig>>) -> Self {
        Self { api_registry }
    }
}

// ============================================================



