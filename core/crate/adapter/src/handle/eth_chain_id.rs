use serde_json::{Value, json};
use std::sync::Arc;

use crate::rpc::JsonRpcError;
use kernel::adapter::{
    IntentSink, {Intent, IntentResult},
};

pub async fn eth_chain_id(intent_sink: Arc<dyn IntentSink>) -> Result<Value, JsonRpcError> {
    match intent_sink.submit(Intent::ChainId).await {
        Ok(IntentResult::U256(v)) => Ok(json!(format!("0x{:x}", v))),
        Ok(_) => Err(JsonRpcError::internal("Invalid chain_id result.")),
        Err(e) => Err(JsonRpcError::internal(format!("{:?}", e))),
    }
}
