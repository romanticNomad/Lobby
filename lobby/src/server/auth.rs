use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use primitives::types::{ApiRegistry, AuthenticatedClient};
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

pub async fn auth_middleware(
    State(api_resigrty): State<ApiRegistry>,
    mut req: Request,
    next: Next,
) -> Result<Response, AuthError> {
    // extracting header
    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or(AuthError::MissingAuthHeader)?;

    // parse bearer payload
    let api_key_elements: Vec<&str> = auth_header
        .strip_prefix("Bearer ")
        .ok_or(AuthError::InvalidAuthFormat)?
        .split(":")
        .collect();

    let api_token = if api_key_elements.len() != 3 {
        return Err(AuthError::InvalidAuthFormat);
    } else {
        api_key_elements[0]
    };

    // lookup api key in DashMap (api_registry) and authenticate
    let client_config = api_resigrty
        .get(api_token)
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
            AuthError::InvalidApiKey => (StatusCode::UNAUTHORIZED, "unauthorized client api-key"),
            AuthError::InvalidAuthFormat => (
                StatusCode::UNAUTHORIZED,
                "invalid authorization format (expected 'Bearer lobby_live_<random_string>:<api_token>:<from_address>')",
            ),
            AuthError::MissingAuthHeader => {
                (StatusCode::UNAUTHORIZED, "missing autherization header")
            }
        };

        warn!("authentication failed: {:?}", self);

        (status, message).into_response()
    }
}

// ============================================================
