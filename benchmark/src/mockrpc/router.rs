use crate::mockrpc::{
    MockRpcState,
    state::{ChainState, NonceUpdateOutcome},
};
use alloy::primitives::Address;
use alloy::{
    consensus::{Transaction, TxEnvelope, transaction::SignerRecoverable},
    eips::eip2718::Decodable2718, // eip2718::Decodable2718 is used instead of rlp::Decodable trait, to respect EIP-2719 type-flag prefix.
    primitives::{Bytes, TxHash}, // important to use alloy::primitives::Bytes and not bytes::Bytes, to allow correct ethereum hex ecoding.
};
use axum::{
    routing::post,
    {Extension, Json, Router},
};
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
    pub inner: HashMap<u64, u16>,
}

impl PortMap {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: HashMap::with_capacity(capacity),
        }
    }

    pub fn insert(&mut self, chain_id: u64, port: u16) {
        self.inner.insert(chain_id, port);
    }
}

// ============================================================
// JSON-RPC wrappers

/// Standard JSON-RPC 2.0 envelope
#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    #[allow(dead_code)]
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcResult {
    TxHash(TxHash),
    ReceiptFound(Arc<Box<RawValue>>),
    ReceiptNotFound(Value),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<RpcResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    pub id: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
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
/// mapped by `port`, does not require `chain_id` indexing.
#[derive(Clone)]
pub struct ChainContext {
    #[allow(dead_code)]
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
#[derive(Clone)]
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
    pub async fn spawn_mockrpc_servers(
        &self,
        cancellation_token: CancellationToken,
    ) -> anyhow::Result<PortMap> {
        let mut port_map = PortMap::with_capacity(self.registry.len());

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

        Ok(port_map)
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

    // // DEBUG: Log the first few bytes as hex to verify content
    // println!(
    //     "Received raw transaction bytes (hex): 0x{}",
    //     hex::encode(&raw_bytes[..std::cmp::min(10, raw_bytes.len())])
    // );

    // decode RLP-encoded signed transaction envelope.
    let envelope = match TxEnvelope::decode_2718(&mut raw_bytes.as_ref()) {
        Ok(env) => env,
        Err(_) => {
            return RpcResponse::invalid_params(
                id,
                "Invalid RLP encoding for eth_sendRawTransaction params",
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
        Some(receipt) => RpcResponse::success(id, RpcResult::ReceiptFound(receipt)),

        // Standard: null for unmined/unknown txs
        None => RpcResponse::success(id, RpcResult::ReceiptNotFound(Value::Null)),
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
// unit tests

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        consensus::{SignableTransaction, TxEip1559, TxEnvelope},
        eips::eip2718::Encodable2718,
        network::TxSignerSync,
        primitives::{Address, Bytes, U256},
        signers::local::PrivateKeySigner,
    };
    use reqwest::Client;
    use serde_json::json;

    /// Helper to spin up 3 mock servers for testing
    async fn setup_servers() -> (PortMap, CancellationToken) {
        // Simulating 3 chains: Ethereum (1), Polygon (137), Hoodi (560048)
        let chain_ids = vec![1, 137, 560048];
        let addresses = vec![Address::ZERO]; // Mock custody addresses
        let state = RpcAppState::new(chain_ids, addresses);
        let cancellation_token = CancellationToken::new();
        let port_map = state
            .spawn_mockrpc_servers(cancellation_token.clone())
            .await
            .unwrap();

        (port_map, cancellation_token)
    }

    #[tokio::test]
    async fn test_mockrpc_servers_lifecycle_and_routing() {
        let (port_map, token) = setup_servers().await;
        let client = Client::new();

        // 1. Verify all 3 servers are listening and routing invalid methods correctly
        for chain_id in [1, 137, 560048] {
            let port = port_map.inner.get(&chain_id).unwrap();
            let url = format!("http://127.0.0.1:{}", port);
            let req = json!({
                "jsonrpc": "2.0",
                "method": "eth_invalidMethod",
                "params": [],
                "id": 1
            });

            let res: RpcResponse = client
                .post(&url)
                .json(&req)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(
                res.error.unwrap().code,
                -32601,
                "Chain {} should return Method Not Found",
                chain_id
            );
        }

        // 2. Test eth_getTransactionReceipt for an unknown hash (should return null)
        let port = port_map.inner.get(&1).unwrap();
        let url = format!("http://127.0.0.1:{}", port);
        let req = json!({
            "jsonrpc": "2.0",
            "method": "eth_getTransactionReceipt",
            "params": ["0x0000000000000000000000000000000000000000000000000000000000000000"],
            "id": 2
        });
        let res: RpcResponse = client
            .post(&url)
            .json(&req)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(res.error.is_none());
        assert!(
            res.result.is_none(),
            "Expected result to be None (JSON null), but got: {:?}",
            res.result
        );

        // 3. Test eth_sendRawTransaction with a valid, dynamically signed EIP-1559 transaction
        // Using Hardhat/Anvil default private key for deterministic testing
        let signer: PrivateKeySigner =
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
                .parse()
                .unwrap();

        let mut tx = TxEip1559 {
            chain_id: 1,
            nonce: 0,
            gas_limit: 21000,
            max_fee_per_gas: 1_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000,
            to: alloy::primitives::TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::new(),
            access_list: Default::default(),
        };

        // Sign the transaction synchronously for the test
        let sig = signer.sign_transaction_sync(&mut tx).unwrap();
        let signed_tx = tx.into_signed(sig);
        let envelope = TxEnvelope::from(signed_tx);
        let raw_bytes = envelope.encoded_2718();
        let hex_bytes = format!("0x{}", hex::encode(raw_bytes));

        let req = json!({
            "jsonrpc": "2.0",
            "method": "eth_sendRawTransaction",
            "params": [hex_bytes],
            "id": 3
        });

        let res: RpcResponse = client
            .post(&url)
            .json(&req)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(
            res.error.is_none(),
            "Expected success, got error: {:?}",
            res.error
        );
        assert!(
            matches!(res.result, Some(RpcResult::TxHash(_))),
            "Expected TxHash result"
        );

        // 4. Cleanup: Gracefully shut down all 3 servers
        token.cancel();

        // Give tokio a moment to process the shutdown signals
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

// ============================================================
