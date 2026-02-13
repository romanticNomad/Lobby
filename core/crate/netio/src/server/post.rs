use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use kernel::{
    traits::IntentRelay,
    types::{
        AuthenticatedClient, ExecutionId, JsonRpcError, JsonRpcErrorResponse, JsonRpcRequest,
        JsonRpcSuccessResponse, RelayHostError, TransactionAcceptedResult,
    },
};
use tracing::{error, info};
use utils::eip1159_normalize::{NormalizationError, normalize_eip1193_transaction};
use uuid::Uuid;

use crate::server::AppState;

// ============================================================
// txn submission errors.

#[derive(Debug)]
pub enum TxnSubmitError {
    InvalidJsonRpcVersion,
    UnsupportedMethod(String),
    MissingParams,
    NormalizationFailed(NormalizationError),
    FromAddressMismatch {
        expected: alloy::primitives::Address,
        actual: alloy::primitives::Address,
    },
    RelayHostError(RelayHostError),
}

// ============================================================
// POST /v1/transactions - Submit EIP-1193 transaction.

pub async fn submit_transaction(
    State(state): State<AppState>,
    Extension(AuthenticatedClient(client_config)): Extension<AuthenticatedClient>,
    Json(request): Json<JsonRpcRequest>,
) -> Result<Json<JsonRpcSuccessResponse>, TxnSubmitError> {
    // validate json rpc
    if request.jsonrpc != "2.0" {
        return Err(TxnSubmitError::InvalidJsonRpcVersion);
    }

    // validate method
    if request.method != "eth_sendTransaction" {
        return Err(TxnSubmitError::UnsupportedMethod(request.method));
    }

    // extract params -> first set in case of a Vector of Eip1193SendTransactionParams.
    let params = request
        .params
        .into_iter()
        .next()
        .ok_or(TxnSubmitError::MissingParams)?;

    // Normalize to lobby Eip1159Transaction
    let (txn, from_address) = normalize_eip1193_transaction(params)
        .map_err(|e| TxnSubmitError::NormalizationFailed(e))?;

    // validate from_address
    if from_address != client_config.from_address {
        return Err(TxnSubmitError::FromAddressMismatch {
            expected: client_config.from_address,
            actual: from_address,
        });
    }

    // execution_id
    let execution_id = ExecutionId(Uuid::new_v4());
    let chain_id_i64: i64 = txn
        .chain_id
        .0
        .try_into()
        .unwrap_or_else(|_| panic!("chain_id invalid"));

    info!(
        execution_id = %execution_id.0,
        client_id = %client_config.client_id,
        chain_id = chain_id_i64,
        "submitting transaction to RelayHost"
    );

    // submit to relayhost
    state
        .relayhost_handle
        .send_transaction(execution_id, txn, client_config)
        .await
        .map_err(|e| TxnSubmitError::RelayHostError(e))?;

    Ok(Json(JsonRpcSuccessResponse {
        jsonrpc: "2.0".to_string(),
        result: TransactionAcceptedResult {
            execution_id,
            status: "accepted".to_string(),
        },
        id: request.id,
    }))
}

// ============================================================
// implimenting IntoResponse for TxnSubmitError

impl IntoResponse for TxnSubmitError {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            TxnSubmitError::InvalidJsonRpcVersion => (
                StatusCode::BAD_REQUEST,
                JsonRpcError {
                    code: -32600,
                    message: "Invalid JSON-RPC version (must be 2.0)".to_string(),
                    data: None,
                },
            ),
            TxnSubmitError::UnsupportedMethod(method) => (
                StatusCode::BAD_REQUEST,
                JsonRpcError {
                    code: -32601,
                    message: format!("Unsupported method: {}", method),
                    data: None,
                },
            ),
            TxnSubmitError::MissingParams => (
                StatusCode::BAD_REQUEST,
                JsonRpcError {
                    code: -32602,
                    message: "Missing transaction parameters".to_string(),
                    data: None,
                },
            ),
            TxnSubmitError::NormalizationFailed(e) => (
                StatusCode::BAD_REQUEST,
                JsonRpcError {
                    code: -32602,
                    message: format!("Invalid params: {}", e),
                    data: None,
                },
            ),
            TxnSubmitError::FromAddressMismatch { expected, actual } => (
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
            TxnSubmitError::RelayHostError(e) => {
                error!("RelayHost error: {:?}", e);
                match e {
                    RelayHostError::DuplicateExecutionId(id) => (
                        StatusCode::CONFLICT,
                        JsonRpcError {
                            code: -32000,
                            message: format!("Duplicate execution_id: {}", id),
                            data: None,
                        },
                    ),
                    RelayHostError::ValidationFailed(msg) => (
                        StatusCode::BAD_REQUEST,
                        JsonRpcError {
                            code: -32602,
                            message: format!("Transaction validation failed: {}", msg),
                            data: None,
                        },
                    ),
                    _ => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        JsonRpcError {
                            code: -32603,
                            message: "Internal server error".to_string(),
                            data: None,
                        },
                    ),
                }
            }
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
