// use std::sync::Arc;

// use axum::{
//     extract::{Request, State},
//     middleware::Next,
//     response::Response,
// };
// use dashmap::DashMap;
// use kernel::types::{ApiKey, ClientConfig};

// // ============================================================
// // middleware - auth state

// #[derive(Clone)]
// pub struct AuthState {
//     api_registry: Arc<DashMap<ApiKey, ClientConfig>>,
// }

// impl AuthState {
//     pub fn new(api_registry: Arc<DashMap<ApiKey, ClientConfig>>) -> Self {
//         Self { api_registry }
//     }
// }

// // ============================================================
// // Authentication errors.

// #[derive(Debug)]
// pub enum AuthError {
//     MissingAuthHeader,
//     InvalidAuthFormat,
//     InvalidApiKey,
// }

// // ============================================================

// pub async fn auth_middleware(
//     State(state): State<AuthState>,
//     mut req: Request,
//     next: Next,
// ) -> Result<Response, AuthError> {
// }
