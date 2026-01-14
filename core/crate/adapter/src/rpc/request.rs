use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use super::{JsonRpcError, JsonRpcResponse};
use crate::handle::eth_chain_id;
use kernel::adapter::IntentSink;

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub async fn filter(
        self,
        intent_sink: Arc<dyn IntentSink>,
    ) -> Result<JsonRpcResponse, JsonRpcResponse> {
        if self.jsonrpc != "2.0".to_string() {
            return Err(JsonRpcResponse::error(
                self.id,
                JsonRpcError::invalid_request("invalid json-rpc version."),
            ));
        }

        let result = match self.method.as_str() {
            "eth_chainId" => eth_chain_id(intent_sink).await,
            _ => Err(JsonRpcError::method_not_found("method not supported.")),
        };

        match result {
            Ok(value) => Ok(JsonRpcResponse::success(self.id, value)),
            Err(e) => Err(JsonRpcResponse::error(self.id, e)),
        }
    }
}
