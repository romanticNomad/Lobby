use std::collections::HashMap;
use std::net::Ipv4Addr;
use crate::mockrpc::state::ChainState;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use axum::{Extension, Json, Router};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::info;
// ============================================================
// type alias

/// Primary app registry for mockrpc.
pub type ChainRegistry = DashMap<u64, Arc<ChainState>>;

// ============================================================
// JSON-RPC wrappers

/// Standard JSON-RPC 2.0 envelope
#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Vec<Value>>,
    #[serde(default)]
    pub id: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    pub id: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
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
// RpcAppState.

/// Primary AppState for mock rpc servers.
///
/// used to build `ChainContext` for individual `chain_id(s)` and spawn respective servers.
pub struct RpcAppState {
    registry: ChainRegistry,
}

impl RpcAppState {
    pub fn new(chain_ids: Vec<u64>, addresses: Vec<String>) -> Self {
        let chain_registry = DashMap::new();
        for chain_id in chain_ids {
            chain_registry.insert(chain_id, Arc::new(ChainState::new(addresses.clone())));
        }

        Self {
            registry: chain_registry,
        }
    }

    /// Spawns isolated Axum servers per chain_id.
    /// Returns `(port_map, shutdown_token)` for `main.rs` orchestration.
    pub async fn spawn_mockrpc_servers(&self) -> (HashMap<u64, u16>, CancellationToken) {
        let mut port_map = HashMap::with_capacity(self.registry.len());
        let cancellation_token = CancellationToken::new();

        for (chain_id, chain_state) in self.registry.into_iter() {
            let chain_context = ChainContext::new(chain_id, chain_state);
            let app = build_router(chain_context.clone());

            // dynamic port allocation by OS
            let listner = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))
                .await
                .expect("[router] failed to bind TcpListner");

            let port = listner.local_addr().unwrap().port();
            port_map.insert(chain_id, port);
            let tocken = cancellation_token.clone();

            tokio::spawn(async move {
               info!(chain_id, port, "Starting mock RPC server");
                axum::serve(listner, app)
                    .with_graceful_shutdown(tocken.cancelled())
                    .await
                    .expect("[router] server exited unexpectedly");
            });
        }

        (port_map, cancellation_token)
    }
}

// ============================================================
// handler function

// TODO: programme the handlers to accomodate nonce validation and RpcResponse generation.
async fn handle_jsonrpc(
    Extension(ctx): Extension<ChainContext>,
    Json(req): Json<RpcRequest>
) -> Response {
    let responce = match req.method.as_str() {
        "eth_sendRawTransaction" => handle_send_raw_transaction(&ctx, &req.params).await,
        "eth_getTransactionReceipt" => handle_get_transaction_receipt(&ctx, &req.params).await,
        _ => RpcResponse::method_not_found(req.id)
    };

    Json(responce).into_response()
}

async fn handle_send_raw_transaction(ctx: &ChainContext, params: &Option<Vec<Value>>) -> RpcResponse {
    // dummy response
    RpcResponse::method_not_found(Some(Value::Null))
}

async fn handle_get_transaction_receipt(ctx: &ChainContext, params: &Option<Vec<Value>>) -> RpcResponse {
    // dummy response
    RpcResponse::method_not_found(Some(Value::Null))
}

// ============================================================
// helper functions

fn build_router(ctx: ChainContext) -> Router  {
    Router::new()
        .route("/", post(handle_jsonrpc))
        .layer(Extension(ctx))
}

// ============================================================
