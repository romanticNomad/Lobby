use crate::mockrpc::{
    MockRpcState,
    state::{ChainState, NonceUpdateOutcome},
};
use alloy::primitives::Address;
use alloy::{
    consensus::{Transaction, TxEnvelope, transaction::SignerRecoverable},
    primitives::TxHash,
    rlp::Decodable,
};
use axum::{
    routing::post,
    {Extension, Json, Router},
};
use bytes::Bytes;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::value::RawValue;
use std::{collections::HashMap, net::Ipv4Addr, sync::Arc};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

// ============================================================
// type alias

/// Primary app registry for mockrpc.
pub type ChainRegistry = DashMap<u64, Arc<ChainState>>;

// ============================================================
// port_map

/// std Hasmap for mapping `chain_id` to respective `ports`
#[derive(Debug)]
pub struct PortMap {
    port_map: HashMap<u64, u16>,
}

impl PortMap {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            port_map: HashMap::with_capacity(capacity),
        }
    }

    pub fn insert(&mut self, chain_id: u64, port: u16) {
        self.port_map.insert(chain_id, port);
    }
}

// ============================================================
// JSON-RPC wrappers

/// Standard JSON-RPC 2.0 envelope
#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub id: Option<Value>,
}

/// Enum constituting possible result from mockrpc handlers
/// * `TxHash` for `eth_sentRawTrasnsaction` request.
/// * `Arc<Box<RawValue>>` when `eth_getTransactionReciept` returns a receipt.
/// * `Value::None` when `eth_getTransactionReciept` does not return a receipt.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum RpcResult {
    TxHash(TxHash),
    RecieptFound(Arc<Box<RawValue>>),
    RecieptNotFound(Value),
}

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<RpcResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    pub id: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

/// eth_sendRawTransaction: ["0x<rlp_encoded_signed_tx>"]
#[derive(Debug, Deserialize)]
pub struct SendRawTransactionParams(pub Vec<Bytes>);

/// eth_getTransactionReceipt: ["0x<tx_hash>"]
#[derive(Debug, Deserialize)]
pub struct GetTransactionReceiptParams(pub Vec<TxHash>);

impl RpcResponse {
    pub fn success(id: Option<Value>, result: RpcResult) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result: Some(result),
            error: None,
            id,
        }
    }
    pub fn error(id: Option<Value>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
            id,
        }
    }
    pub fn method_not_found(id: Option<Value>) -> Self {
        Self::error(id, -32601, "Method not found")
    }

    pub fn invalid_params(id: Option<Value>, message: impl Into<String>) -> Self {
        Self::error(id, -32602, message)
    }
}

// ============================================================
// router contexct

/// `chain` scoped router context for the axum server.
#[derive(Clone)]
pub struct ChainContext {
    chain_id: u64,
    chain_state: Arc<ChainState>,
}

impl ChainContext {
    pub fn new(chain_id: u64, chain_state: Arc<ChainState>) -> Self {
        Self {
            chain_id,
            chain_state,
        }
    }
}

// ============================================================
// RpcAppState. and server implimentation

/// Primary AppState for mock rpc servers.
///
/// used to build `ChainContext` for individual `chain_id(s)` and spawn respective servers.
pub struct RpcAppState {
    registry: Arc<ChainRegistry>,
}

impl RpcAppState {
    pub fn new(chain_ids: Vec<u64>, addresses: Vec<Address>) -> Self {
        let chain_registry = DashMap::new();
        for chain_id in chain_ids {
            chain_registry.insert(chain_id, Arc::new(ChainState::new(addresses.clone())));
        }

        Self {
            registry: Arc::new(chain_registry),
        }
    }

    /// Spawns isolated Axum servers per chain_id.
    ///
    /// Returns `(port_map, shutdown_token)` for `main.rs` orchestration.
    pub async fn spawn_mockrpc_servers(&self) -> anyhow::Result<(PortMap, CancellationToken)> {
        let mut port_map = PortMap::with_capacity(self.registry.len());
        let cancellation_token = CancellationToken::new();

        for map_ref in self.registry.iter() {
            let (chain_id, chain_state) = (map_ref.key().clone(), map_ref.value().clone());
            let chain_context = ChainContext::new(chain_id, chain_state);
            let app = build_router(chain_context);

            let listner = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
            let port = listner.local_addr()?.port();
            port_map.insert(chain_id, port);

            info!(chain_id, port, "Mockrpc server online");

            let token = cancellation_token.clone();
            tokio::spawn(async move {
                axum::serve(listner, app)
                    .with_graceful_shutdown(async move {
                        token.cancelled().await;
                    })
                    .await
                    .unwrap_or_else(|e| warn!(chain_id, port, "Failed to spawn server: {}", e));

                info!(chain_id, port, "Mockrpc server shut down");
            });
        }

        Ok((port_map, cancellation_token))
    }
}

// ============================================================
// handler function

async fn handle_jsonrpc(
    Extension(ctx): Extension<ChainContext>,
    Json(req): Json<RpcRequest>,
) -> Json<RpcResponse> {
    let responce = match req.method.as_str() {
        "eth_sendRawTransaction" => handle_send_raw_transaction(&ctx, req.params, req.id).await,
        "eth_getTransactionReceipt" => {
            handle_get_transaction_receipt(&ctx, req.params, req.id).await
        }
        _ => RpcResponse::method_not_found(req.id),
    };

    Json(responce)
}

async fn handle_send_raw_transaction(
    ctx: &ChainContext,
    params: Option<Value>,
    id: Option<Value>,
) -> RpcResponse {
    let payload =
        match serde_json::from_value::<SendRawTransactionParams>(params.unwrap_or(Value::Null)) {
            Ok(payload) if !payload.0.is_empty() => payload,
            _ => return RpcResponse::invalid_params(id, "Invalid sendRawTransaction params."),
        };
    let raw_bytes = &payload.0[0];

    // decode RLP-encoded signed transaction envelope.
    let envelope = match TxEnvelope::decode(&mut raw_bytes.as_ref()) {
        Ok(env) => env,
        Err(_) => {
            return RpcResponse::invalid_params(
                id,
                "Invalid RLP encoding to eth_sendRawTransaction params",
            );
        }
    };

    // extract nonce, and recover sender's address
    let rlp_nonce = envelope.nonce();
    let from_address = match envelope.recover_signer() {
        Ok(address) => address,
        Err(_) => return RpcResponse::invalid_params(id, "Failed to recover signature"),
    };

    // generate tx_hash
    let tx_hash = envelope.hash().clone();

    // final nonce validation and update.
    match ctx.chain_state.update_nonce(from_address, rlp_nonce) {
        NonceUpdateOutcome::NonceAdvanced(_new_nonce) => {
            // update the receipt_collection with the new txhash
            ctx.chain_state.update_receipt(tx_hash);

            RpcResponse::success(id, RpcResult::TxHash(tx_hash))
        }
        NonceUpdateOutcome::NonceTooLow => {
            warn!("NonceTooLow rejected");
            RpcResponse::error(id, -32000, "nonce too low")
        }
    }
}

async fn handle_get_transaction_receipt(
    ctx: &ChainContext,
    params: Option<Value>,
    id: Option<Value>,
) -> RpcResponse {
    let payload =
        match serde_json::from_value::<GetTransactionReceiptParams>(params.unwrap_or_default()) {
            Ok(payload) if !payload.0.is_empty() => payload,
            _ => {
                return RpcResponse::invalid_params(id, "Invalid GetTransactionReceipt params.");
            }
        };
    let tx_hash = payload.0[0];

    match ctx.chain_state.fetch_receipt(&tx_hash) {
        // receipt found, returns boxed RawValue
        Some(receipt) => RpcResponse::success(id, RpcResult::RecieptFound(receipt)),

        // Standard: null for unmined/unknown txs
        None => RpcResponse::success(id, RpcResult::RecieptNotFound(Value::Null)),
    }
}

// ============================================================
// helper functions

fn build_router(ctx: ChainContext) -> Router {
    Router::new()
        .route("/", post(handle_jsonrpc))
        .layer(Extension(ctx))
}

// ============================================================
