use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use super::response;
use crate::handle;
use kernel::adapter::IntentSink;

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    pub params: Option<Value>,
}
