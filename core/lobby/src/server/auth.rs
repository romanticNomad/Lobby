use crate::server::AppState;
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use kernel::types::AuthenticatedClient;
use tracing::warn;

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

pub(crate) async fn auth_middleware(
    State(state): State<AppState>,
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

// ============================================================
// IntoResponse implintations for the used errors

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AuthError::InvalidApiKey => (StatusCode::UNAUTHORIZED, "missing autherization header"),
            AuthError::InvalidAuthFormat => (
                StatusCode::UNAUTHORIZED,
                "invalid authorization format (expected 'Bearer <token>')",
            ),
            AuthError::MissingAuthHeader => (StatusCode::UNAUTHORIZED, "invalid API key"),
        };

        warn!("authentication failed: {:?}", self);

        (status, message).into_response()
    }
}

// ============================================================
