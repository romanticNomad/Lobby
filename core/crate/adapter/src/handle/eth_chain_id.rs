use serde_json::{Value, json};
use std::sync::Arc;

use crate::rpc::JsonRpcError;
use kernel::adapter::{
    Pipeline, {Intent, IntentResult},
};

pub async fn eth_chain_id(pipeline: Arc<dyn Pipeline>) -> Result<Value, JsonRpcError> {
    match pipeline.submit(Intent::ChainId).await {
        Ok(IntentResult::Id(v)) => Ok(json!(format!("0x{:x}", v))),
        Ok(_) => Err(JsonRpcError::internal("Invalid chain_id result.")),
        Err(e) => Err(JsonRpcError::internal(format!("{:?}", e))),
    }
}
