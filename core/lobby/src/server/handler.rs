use crate::server::AppState;
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use cortex::{
    error::CortexError,
    state::{JsonStatusResponse, StatusRegistry},
};
use kernel::types::{
    AuthenticatedClient, Eip1193SendTransactionParams, ExecutionId, JsonRpcError,
    JsonRpcErrorResponse, JsonRpcRequest, JsonRpcSuccessResponse, TxnAcceptedResult,
};
use std::sync::Arc;
use tracing::{error, info};
use utils::eip1559::NormalizationError;
use utils::eip1559::normalize_eip1193_transaction;
use uuid::Uuid;

// ============================================================
// txn submission errors.

#[derive(Debug)]
pub enum HandlerError {
    InvalidJsonRpcVersion,
    UnsupportedMethod(String),
    CorruptedParams,
    NormalizationFailed(NormalizationError),
    FromAddressMismatch {
        expected: alloy::primitives::Address,
        actual: alloy::primitives::Address,
    },
    CortexError(CortexError),
    IdParsingError(String),
    RecordNotFound(String),
}

// ============================================================

/// `POST /submit`
///
/// Submits an EIP-1193 transaction for processing through the Cortex pipeline.
/// The transaction is validated, normalized, and handed off to the orchestrator
/// for nonce assignment, signing, broadcasting, and confirmation.
///
/// # Responses
/// - `200 OK` — transaction accepted, returns execution_id and "accepted" status
/// - `400 Bad Request` — invalid JSON-RPC version, unsupported method, or malformed params
/// - `403 Forbidden` — from_address does not match authenticated account
/// - `500 Internal Server Error` — internal pipeline error
pub async fn submit_transaction(
    State(state): State<AppState>,
    Extension(AuthenticatedClient(client_config)): Extension<AuthenticatedClient>,
    Json(request): Json<JsonRpcRequest>,
) -> Result<Json<JsonRpcSuccessResponse>, HandlerError> {
    // Parse and validate the JSON-RPC method

    if request.method != "eth_sendTransaction" {
        return Err(HandlerError::UnsupportedMethod(request.method.clone()));
    }

    // Deserialize and validate EIP-1193 params

    let params: Eip1193SendTransactionParams = request
        .params
        .get(0)
        .cloned()
        .ok_or(HandlerError::CorruptedParams)?;

    // Normalize to lobby Eip1159Transaction

    let (txn, from_address) =
        normalize_eip1193_transaction(params).map_err(|e| HandlerError::NormalizationFailed(e))?;

    // validate from_address

    if from_address != client_config.from_address {
        return Err(HandlerError::FromAddressMismatch {
            expected: client_config.from_address,
            actual: from_address,
        });
    }

    // generate execution_id

    let execution_id = ExecutionId(Uuid::new_v4());

    info!(
        %execution_id,
        from = %client_config.from_address,
        chain_id = %txn.chain_id,
        "transaction submission accepted"
    );

    // hand off to orchestrator

    // This call:
    //   a) Calls relay_host.send_transaction() to persist the intent.
    //   b) Acquires a pipeline semaphore permit.
    //   c) Spawns the Nonce → Sign → Broadcast → Validator pipeline.
    //   d) Returns immediately.

    state
        .cortex_handler
        .submit(execution_id, txn, client_config)
        .await
        .map_err(|e| HandlerError::CortexError(e))?;

    // respond immidiately

    Ok(Json(JsonRpcSuccessResponse {
        jsonrpc: "2.0".to_string(),
        result: TxnAcceptedResult {
            execution_id,
            status: "accepted".to_string(),
        },
        id: request.id,
    }))
}

// ============================================================

/// `GET /status/:execution_id`
///
/// Returns the current pipeline status for an execution. Clients should poll
/// this until status is `confirmed` or `failed`.
///
/// # Responses
/// - `200 OK` — known execution_id, returns `JsonStatusResponse`
/// - `400 Bad Request` — `execution_id` is not a valid UUID
/// - `404 Not Found` — execution_id is unknown (not yet submitted or expired)
pub async fn get_transaction_status(
    State(registry): State<Arc<StatusRegistry>>,
    Path(raw_id): Path<String>,
) -> Result<Json<JsonStatusResponse>, HandlerError> {
    // parse execution_id
    let uuid =
        Uuid::parse_str(&raw_id).map_err(|_| HandlerError::IdParsingError(raw_id.clone()))?;

    let execution_id = ExecutionId(uuid);

    match registry.get(&execution_id) {
        Some(status) => Ok(Json(JsonStatusResponse {
            execution_id: raw_id,
            status,
        })),
        None => Err(HandlerError::RecordNotFound(raw_id)),
    }
}

// ============================================================
// implimenting IntoResponse for HandlerError

impl IntoResponse for HandlerError {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            HandlerError::InvalidJsonRpcVersion => (
                StatusCode::BAD_REQUEST,
                JsonRpcError {
                    code: -32600,
                    message: "Invalid JSON-RPC version (must be 2.0)".to_string(),
                    data: None,
                },
            ),
            HandlerError::UnsupportedMethod(method) => (
                StatusCode::BAD_REQUEST,
                JsonRpcError {
                    code: -32601,
                    message: format!("Unsupported method: {}", method),
                    data: None,
                },
            ),
            HandlerError::CorruptedParams => (
                StatusCode::BAD_REQUEST,
                JsonRpcError {
                    code: -32602,
                    message: "Missing transaction parameters".to_string(),
                    data: None,
                },
            ),
            HandlerError::NormalizationFailed(e) => (
                StatusCode::BAD_REQUEST,
                JsonRpcError {
                    code: -32602,
                    message: format!("Invalid params: {}", e),
                    data: None,
                },
            ),
            HandlerError::FromAddressMismatch { expected, actual } => (
                StatusCode::FORBIDDEN,
                JsonRpcError {
                    code: -32000,
                    message: "From address does not match authenticated account".to_string(),
                    data: Some(serde_json::json!({
                        "expected": format!("{:?}", expected),
                        "actual": format!("{:?}", actual),
                    })),
                },
            ),
            HandlerError::CortexError(e) => {
                error!("orchestration: {:?}", e);
                match e {
                    CortexError::RelayHost(err) => (
                        StatusCode::BAD_REQUEST,
                        JsonRpcError {
                            code: -32602,
                            message: format!("Transaction validation failed: {}", err),
                            data: None,
                        },
                    ),
                    _ => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        JsonRpcError {
                            code: -32603,
                            message: "Internal pipeline error".to_string(),
                            data: None,
                        },
                    ),
                }
            }
            HandlerError::IdParsingError(raw_id) => (
                StatusCode::BAD_REQUEST,
                JsonRpcError {
                    code: -32602,
                    message: format!("{} is not a valid UUID", raw_id),
                    data: None,
                },
            ),
            HandlerError::RecordNotFound(raw_id) => (
                StatusCode::NOT_FOUND,
                JsonRpcError {
                    code: -32001,
                    message: format!("no pipeline record found the give execution id: {}", raw_id),
                    data: None,
                },
            ),
        };

        let response = JsonRpcErrorResponse {
            jsonrpc: "2.0".to_string(),
            error,
            id: serde_json::Value::Null,
        };

        (status, Json(response)).into_response()
    }
}

// ============================================================
