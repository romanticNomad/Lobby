use alloy_primitives::{Address, Bytes, U256};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use kernel::{
    traits::Pipeline,
    types::{ChainId, Intent, IntentResult, SendTransactionIntent, TxNonce},
};

use crate::rpc::JsonRpcError;

#[derive(Debug, Deserialize)]
struct TxParams {
    from: Address,
    to: Option<Address>,
    value: Option<U256>,
    data: Option<Bytes>,
    gas: Option<U256>,
    gas_price: Option<U256>,
    max_fee_per_gas: Option<U256>,
    max_priority_fee_per_gas: Option<U256>,
    nonce: TxNonce,
    chain_id: ChainId,
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
        max_fee_per_gas: tx.max_fee_per_gas,
        max_priority_fee_per_gas: tx.max_priority_fee_per_gas,
        nonce: tx.nonce,
        chain_id: tx.chain_id,
    });

    // sending normalized data: intent, into the pipeine.
    match pipeline.submit(intent).await {
        Ok(IntentResult::TxHash(hash)) => Ok(json!(format!("0x{}", hex::encode(hash)))),
        Err(e) => Err(JsonRpcError::internal(format!("{:?}", e))),
    }
}
