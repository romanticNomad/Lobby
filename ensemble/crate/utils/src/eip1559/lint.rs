/// this module only consists of linting functions that do not require
/// database lookup .i.e, it only check business logic for eip1159
/// transaction fields.
use alloy::primitives::U256;
use kernel::types::{Eip1559Transaction, RelayHostError};

// ============================================================

pub fn transaction_lint(txn: &Eip1559Transaction) -> Result<(), RelayHostError> {
    // ============================================================
    // gas limit check

    if txn.gas_limit == U256::ZERO {
        return Err(RelayHostError::ValidationFailed(
            "gas limit cannot be zero".to_string(),
        ));
    };

    // ============================================================
    // max gas limit (30M gas)

    const MAX_GAS_LIMIT: u64 = 30_000_000;
    if txn.gas_limit > U256::from(MAX_GAS_LIMIT) {
        return Err(RelayHostError::ValidationFailed(format!(
            "lint error: fee exeeds max gas limit: {}",
            MAX_GAS_LIMIT
        )));
    };

    // ============================================================
    // fee validation

    if txn.max_fee_per_gas == U256::ZERO {
        return Err(RelayHostError::ValidationFailed(
            "lint error: max_fee_per_gas cannot be zero".to_string(),
        ));
    };

    if txn.max_priority_fee_per_gas >= txn.max_fee_per_gas {
        return Err(RelayHostError::ValidationFailed(
            "lint error: max_priority_fee_per_gas cannot exceed max_fee_per_gas".to_string(),
        ));
    };

    // ============================================================
    // chain_id validation

    const SUPPORTED_CHAINS: &[u64] = &[
        1,      // Ethereum Mainnet
        5,      // Goerli
        560048, // Hoodi
        137,    // Polygon
        42161,  // Arbitrum One
    ];

    if !SUPPORTED_CHAINS.contains(&(txn.chain_id.0).try_into().map_err(|_| {
        RelayHostError::ValidationFailed("lint error: invalid chain_id format".to_string())
    })?) {
        return Err(RelayHostError::ValidationFailed(
            "lint error: unsupported chain_id".to_string(),
        ));
    };

    // ============================================================

    Ok(())
}
