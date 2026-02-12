use std::sync::Arc;

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use dashmap::DashMap;
use kernel::types::{ApiKey, AuthenticatedClient, ClientConfig};

// ============================================================
// middleware - auth state

#[derive(Clone)]
pub struct AuthState {
    api_registry: Arc<DashMap<ApiKey, ClientConfig>>,
}

impl AuthState {
    pub fn new(api_registry: Arc<DashMap<ApiKey, ClientConfig>>) -> Self {
        Self { api_registry }
    }
}

// ============================================================
// authentication errors.

#[derive(Debug)]
pub enum AuthError {
    MissingAuthHeader,
    InvalidAuthFormat,
    InvalidApiKey,
}

// ============================================================
// authentication and responce handeling

pub async fn auth_middleware(
    State(state): State<AuthState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AuthError> {
    // extracting header
    let auth_header = req
    .headers()
    .get(axum::http::header::AUTHORIZATION)
    .and_then(|h| h.to_str().ok())
    .ok_or(AuthError::MissingAuthHeader)?;

    // parse bearer tocken
    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(AuthError::InvalidAuthFormat)?;

    // lookup api key in DashMap (api_registry)
    let client_config = state
        .api_registry
        .get(token)
        .map(|entry| entry.value().clone())
        .ok_or(AuthError::InvalidApiKey)?;

    // attach ClientConfig to the request
    req.extensions_mut()
        .insert(AuthenticatedClient(client_config));
    
    Ok(next.run(req).await)
}
