// use axum::{Extension, Json, extract::State, http::StatusCode, response::{IntoResponse, Response}};
// use cortex::error::CortexError;
// use kernel::types::{AuthenticatedClient, Eip1193SendTransactionParams, ExecutionId, JsonRpcError, JsonRpcErrorResponse, JsonRpcRequest, JsonRpcSuccessResponse, TxnAcceptedResult};
// use utils::eip1559::NormalizationError;
// use tracing::{error, info};
// use utils::eip1559::normalize_eip1193_transaction;
// use uuid::Uuid;

// use crate::server::AppState;

// // ============================================================
// // txn submission errors.

// #[derive(Debug)]
// pub enum TxnSubmitError {
//     InvalidJsonRpcVersion,
//     UnsupportedMethod(String),
//     CorruptedParams,
//     NormalizationFailed(NormalizationError),
//     FromAddressMismatch {
//         expected: alloy::primitives::Address,
//         actual: alloy::primitives::Address,
//     },
//     CortexError(CortexError),
// }

// // ============================================================

// pub async fn submit_transaction(
//     State(state): State<AppState>,
//     Extension(AuthenticatedClient(client_config)): Extension<AuthenticatedClient>,
//     Json(request): Json<JsonRpcRequest>
// ) -> Result<Json<JsonRpcSuccessResponse>, TxnSubmitError> {
//     // Parse and validate the JSON-RPC method

//     if request.method != "eth_sendTransaction" {
//         return Err(TxnSubmitError::UnsupportedMethod(request.method.clone()));
//     }

//     // Deserialize and validate EIP-1193 params

//     let params: Eip1193SendTransactionParams = serde_json::from_value(
//         request.params
//             .get(0)
//             .and_then(|v| serde_json::from_value(v.clone().into()).ok())
//             .ok_or(TxnSubmitError::CorruptedParams)?,
//     )
//     .map_err(|_| TxnSubmitError::CorruptedParams)?;

//     // Normalize to lobby Eip1159Transaction

//     let (txn, from_address) = normalize_eip1193_transaction(params)
//     .map_err(|e| TxnSubmitError::NormalizationFailed(e))?;

//     // validate from_address

//     if from_address != client_config.from_address {
//         return Err(TxnSubmitError::FromAddressMismatch {
//             expected: client_config.from_address,
//             actual: from_address,
//         });
//     }

//     // generate execution_id

//     let execution_id = ExecutionId(Uuid::new_v4());

//     info!(
//         %execution_id,
//         from = %client_config.from_address,
//         chain_id = %txn.chain_id,
//         "transaction submission accepted"
//     );

//     // hand off to orchestrator

//     // This call:
//     //   a) Calls relay_host.send_transaction() to persist the intent.
//     //   b) Acquires a pipeline semaphore permit.
//     //   c) Spawns the Nonce → Sign → Broadcast → Validator pipeline.
//     //   d) Returns immediately.

//     state
//         .cortex_handler
//         .submit(execution_id, txn, client_config)
//         .await
//         .map_err(|e| TxnSubmitError::CortexError(e))?;

//     // respond immidiately

//     Ok(Json(JsonRpcSuccessResponse{
//         jsonrpc: "2.0".to_string(),
//         result: TxnAcceptedResult {
//             execution_id,
//             status: "accepted".to_string(),
//         },
//         id: request.id
//     }))
// }

// // ============================================================
// // implimenting IntoResponse for TxnSubmitError

// impl IntoResponse for TxnSubmitError {
//     fn into_response(self) -> Response {
//         let (status, error) = match self {
//             TxnSubmitError::InvalidJsonRpcVersion => (
//                 StatusCode::BAD_REQUEST,
//                 JsonRpcError {
//                     code: -32600,
//                     message: "Invalid JSON-RPC version (must be 2.0)".to_string(),
//                     data: None,
//                 },
//             ),
//             TxnSubmitError::UnsupportedMethod(method) => (
//                 StatusCode::BAD_REQUEST,
//                 JsonRpcError {
//                     code: -32601,
//                     message: format!("Unsupported method: {}", method),
//                     data: None,
//                 },
//             ),
//             TxnSubmitError::CorruptedParams => (
//                 StatusCode::BAD_REQUEST,
//                 JsonRpcError {
//                     code: -32602,
//                     message: "Missing transaction parameters".to_string(),
//                     data: None,
//                 },
//             ),
//             TxnSubmitError::NormalizationFailed(e) => (
//                 StatusCode::BAD_REQUEST,
//                 JsonRpcError {
//                     code: -32602,
//                     message: format!("Invalid params: {}", e),
//                     data: None,
//                 },
//             ),
//             TxnSubmitError::FromAddressMismatch { expected, actual } => (
//                 StatusCode::FORBIDDEN,
//                 JsonRpcError {
//                     code: -32000,
//                     message: "From address does not match authenticated account".to_string(),
//                     data: Some(serde_json::json!({
//                         "expected": format!("{:?}", expected),
//                         "actual": format!("{:?}", actual),
//                     })),
//                 },
//             ),
//             TxnSubmitError::CortexError(e) => {
//                 error!("RelayHost error: {:?}", e);
//                 match e {
//                     CortexError::RelayHost(err) => (
//                         StatusCode::BAD_REQUEST,
//                         JsonRpcError {
//                             code: -32602,
//                             message: format!("Transaction validation failed: {}", err),
//                             data: None,
//                         },
//                     ),
//                     _ => (
//                         StatusCode::INTERNAL_SERVER_ERROR,
//                         JsonRpcError {
//                             code: -32603,
//                             message: "Internal server error".to_string(),
//                             data: None,
//                         },
//                     ),
//                 }
//             }
//         };

//         let response = JsonRpcErrorResponse {
//             jsonrpc: "2.0".to_string(),
//             error,
//             id: serde_json::Value::Null,
//         };

//         (status, Json(response)).into_response()
//     }
// }

// // ============================================================
