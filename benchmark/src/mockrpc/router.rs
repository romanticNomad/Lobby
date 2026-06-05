use crate::mockrpc::{ChainRegistry, state::ChainState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use dashmap::DashMap;
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
// Context for axum app (Shared state among handlers)

#[derive(Clone)]
pub struct ChainContext {
    chain_id: u64,
    rpc_state: Arc<ChainState>,
}

impl ChainContext {
    pub fn new(chain_id: u64, addresses: Vec<String>) -> Self {
        let rpc_state = Arc::new(ChainState::new(addresses));

        Self {
            chain_id,
            rpc_state,
        }
    }
}

// ============================================================
// Mock Rpc AppState.

pub struct RpcAppState {
    registry: Arc<ChainRegistry>,
}

// impl RpcAppState {
//     pub fn new(chain_ids: Vec<u64>, addresses: Vec<String>) -> Self {
//         let chain_registry = DashMap::new();
//         for chain_id in chain_ids {
//             chain_registry.insert(chain_id, Arc::new(ChainState::new(addresses)));
//         };
//     }
// }
