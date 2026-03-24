use alloy::primitives::{Address, U256, bytes::Bytes};
use primitives::types::{ChainId, Eip1193SendTransactionParams, Eip1559Transaction, TxNonce};
use thiserror::Error;

// ============================================================
// errors

#[derive(Debug, Error)]
pub enum NormalizationError {
    #[error("Invalid field: {0}")]
    InvalidField(String),
}

// ============================================================
// lobby normalization for eip1193 txn requests

pub fn normalize_eip1193_transaction(
    params: Eip1193SendTransactionParams,
) -> Result<(Eip1559Transaction, Address), NormalizationError> {
    // Parse chain_id
    let chain_id_u64 = parse_hex_u64(&params.chain_id)
        .map_err(|_| NormalizationError::InvalidField("chain_id".to_string()))?;

    let chain_id = ChainId::try_from(chain_id_u64 as i64).unwrap();

    // Parse gas parameters (default to 0, will be estimated later)
    let gas_limit = params
        .gas
        .as_ref()
        .map(|s| parse_hex_u256(s))
        .transpose()
        .map_err(|_| NormalizationError::InvalidField("gas".to_string()))?
        .unwrap_or(U256::ZERO);

    let max_fee_per_gas = params
        .max_fee_per_gas
        .as_ref()
        .map(|s| parse_hex_u256(s))
        .transpose()
        .map_err(|_| NormalizationError::InvalidField("max_fee_per_gas".to_string()))?
        .unwrap_or(U256::ZERO);

    let max_priority_fee_per_gas = params
        .max_priority_fee_per_gas
        .as_ref()
        .map(|s| parse_hex_u256(s))
        .transpose()
        .map_err(|_| NormalizationError::InvalidField("max_priority_fee_per_gas".to_string()))?
        .unwrap_or(U256::ZERO);

    // Parse value (default to 0)
    let value = params
        .value
        .as_ref()
        .map(|s| parse_hex_u256(s))
        .transpose()
        .map_err(|_| NormalizationError::InvalidField("value".to_string()))?
        .unwrap_or(U256::ZERO);

    // Parse data (default to empty bytes)
    let data = params
        .data
        .as_ref()
        .map(|s| parse_hex_bytes(s))
        .transpose()
        .map_err(|_| NormalizationError::InvalidField("data".to_string()))?
        .unwrap_or_default();

    // Parse access list
    let access_list = params
        .access_list
        .unwrap_or_default()
        .into_iter()
        .map(|item| {
            let storage_keys = item
                .storage_keys
                .into_iter()
                .map(|key| parse_hex_u256(&key))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| NormalizationError::InvalidField("access_list".to_string()))?;
            Ok((item.address, storage_keys))
        })
        .collect::<Result<Vec<_>, NormalizationError>>()?;

    let transaction = Eip1559Transaction {
        chain_id,
        nonce: TxNonce::try_from(0).unwrap(), // Will be assigned by Nonce actor
        max_priority_fee_per_gas,
        max_fee_per_gas,
        gas_limit,
        to: params.to,
        value,
        data,
        access_list,
    };

    Ok((transaction, params.from))
}

// ============================================================
// parsing functions.

/// Parse hex string to u64.
fn parse_hex_u64(s: &str) -> Result<u64, ()> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(s, 16).map_err(|_| ()) // error itself is irrelevent, only an indication is required.
}

/// Parse hex string to U256.
fn parse_hex_u256(s: &str) -> Result<U256, ()> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    U256::from_str_radix(s, 16).map_err(|_| ()) // error itself is irrelevent, only an indication is required.
}

/// Parse hex string to Bytes.
fn parse_hex_bytes(s: &str) -> Result<Bytes, hex::FromHexError> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(s)?;
    Ok(Bytes::from(bytes))
}

// ============================================================
