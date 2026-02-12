use axum::{Extension, Json, extract::State};
use kernel::{
    traits::IntentRelay,
    types::{
        AuthenticatedClient, ExecutionId, JsonRpcRequest, JsonRpcSuccessResponse, RelayHostError,
        TransactionAcceptedResult,
    },
};
use tracing::info;
use utils::eip1159_normalize::{NormalizationError, normalize_eip1193_transaction};
use uuid::Uuid;

use crate::server::AppState;

// ============================================================
// txn submission errors.

#[derive(Debug)]
pub enum TransactionError {
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
) -> Result<Json<JsonRpcSuccessResponse>, TransactionError> {
    // validate json rpc
    if request.jsonrpc != "2.0" {
        return Err(TransactionError::InvalidJsonRpcVersion);
    }

    // validate method
    if request.method != "eth_sendTransaction" {
        return Err(TransactionError::UnsupportedMethod(request.method));
    }

    // extract params -> first set in case of a Vector of Eip1193SendTransactionParams.
    let params = request
        .params
        .into_iter()
        .next()
        .ok_or(TransactionError::MissingParams)?;

    // Normalize to lobby Eip1159Transaction
    let (txn, from_address) = normalize_eip1193_transaction(params)
        .map_err(|e| TransactionError::NormalizationFailed(e))?;

    // validate from_address
    if from_address != client_config.from_address {
        return Err(TransactionError::FromAddressMismatch {
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
        .map_err(|e| TransactionError::RelayHostError(e))?;

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
