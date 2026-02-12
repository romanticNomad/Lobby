// use axum::{Extension, Json, extract::State};
// use kernel::types::{AuthenticatedClient, JsonRpcRequest, JsonRpcSuccessResponse, RelayHostError};
// use utils::eip1159_normalize::NormalizationError;

// use crate::server::AppState;

// // ============================================================
// // txn submission errors.

// #[derive(Debug)]
// pub enum TransactionError {
//     InvalidJsonRpcVersion,
//     UnsupportedMethod(String),
//     MissingParams,
//     NormalizationFailed(NormalizationError),
//     FromAddressMismatch {
//         expected: alloy::primitives::Address,
//         actual: alloy::primitives::Address,
//     },
//     RelayHostError(RelayHostError),
// }

// // ============================================================
// // POST /v1/transactions - Submit EIP-1193 transaction.

// pub async fn submit_transaction(
//     State(state): State<AppState>,
//     Extension(AuthenticatedClient(client_config)): Extension<AuthenticatedClient>,
//     Json(request): Json<JsonRpcRequest>,
// ) -> Result<Json<JsonRpcSuccessResponse>, TransactionError> {
    
// }

// // ============================================================
