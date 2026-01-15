use alloy_primitives::{Address, Bytes, U256};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use kernel::adapter::{Intent, IntentResult, Pipeline, SendTransactionIntent};

use crate::rpc::JsonRpcError;

#[derive(Debug, Deserialize)]
struct TxParams {
    from: Address,
    to: Option<Address>,
    value: Option<U256>,
    data: Option<Bytes>,
    gas: Option<U256>,
    gas_price: Option<U256>,
    nonce: Option<U256>,
    chain_id: Option<U256>,
}

pub async fn eth_send_transaction(
    pipeline: Arc<dyn Pipeline>,
    params: Option<Value>,
) -> Result<Value, JsonRpcError> {
    let params = params.ok_or_else(|| JsonRpcError::invalid_params("missing params"))?;

    let list: Vec<TxParams> = serde_json::from_value(params)
        .map_err(|e| JsonRpcError::invalid_params(format!("invalid params: {e}")))?;

    let tx = list
        .get(0)
        .ok_or_else(|| JsonRpcError::invalid_params("invalid transaction params"))?;

    let intent = Intent::SendTransaction(SendTransactionIntent {
        from: tx.from,
        to: tx.to,
        value: tx.value.unwrap_or_default(),
        data: tx.data.clone().unwrap_or_default(), // Bytes is not Copy;
        gas: tx.gas,
        gas_price: tx.gas_price,
        nonce: tx.nonce,
        chain_id: tx.chain_id,
    });

    match pipeline.submit(intent).await {
        Ok(IntentResult::Bytes(hash)) => Ok(json!(format!("0x{}", hex::encode(hash)))),
        Ok(_) => Err(JsonRpcError::internal("Invalid result type")),
        Err(e) => Err(JsonRpcError::internal(format!("{:?}", e))),
    }
}
