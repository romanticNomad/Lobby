use alloy::{
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
};
use anyhow::{Context, Result};
use primitives::types::{ChainId, ExecutionId, TxNonce};
use sqlx::PgPool;

// ============================================================
// Account Funding via Anvil

/// Fund an account using Anvil's `anvil_setBalance` RPC method.
pub async fn fund_account(rpc_url: &str, address: Address, balance: U256) -> Result<()> {
    let provider = ProviderBuilder::new().connect_http(rpc_url.parse()?);

    let params = serde_json::json!([format!("{:?}", address), format!("{:#?}", balance),]);

    provider
        .raw_request::<_, ()>("anvil_setBalance".into(), params)
        .await
        .context(format!("helpers: failed to set balance for: {:?}", address))?;

    tracing::debug!("Funded {:?} with {} wei", address, balance);
    Ok(())
}

// ============================================================
// Database Poisoning (Nonce Gap Creation)

/// Poison the database by inserting a stuck 'reserved' nonce that will never be finalized.
/// This creates a nonce gap for testing the sweeper bot.
///
/// Returns the poisoned (execution_id, nonce).
pub async fn poison_nonce_gap(
    db: &PgPool,
    chain_id: ChainId,
    from_address: Address,
    nonce: TxNonce,
) -> Result<ExecutionId> {
    let execution_id = ExecutionId(uuid::Uuid::new_v4());
    let execution_id_bytes = execution_id.0.as_bytes();
    let from_address_bytes = from_address.as_slice();
    let chain_id_i64 = chain_id.0.to::<i64>();
    let nonce_i64 = nonce.0.to::<i64>();

    sqlx::query(
        r#"
        INSERT INTO nonce.nonce_assignments (
            execution_id,
            revision,
            chain_id,
            from_address,
            nonce,
            state,
            created_at,
            updated_at
        )
        VALUES ($1, 1, $2, $3, $4, 'reserved', now(), now())
        "#,
    )
    .bind(execution_id_bytes)
    .bind(chain_id_i64)
    .bind(from_address_bytes)
    .bind(nonce_i64)
    .execute(db)
    .await
    .context("Failed to insert poisoned nonce reservation")?;

    tracing::warn!(
        "Poisoned nonce gap: execution_id={}, chain_id={}, from={:?}, nonce={}",
        execution_id,
        chain_id,
        from_address,
        nonce
    );

    Ok(execution_id)
}

// // ============================================================
// // Mempool Verification

// /// Check if a transaction with the given nonce exists in Anvil's pending mempool.
// pub async fn is_transaction_in_mempool(
//     rpc_url: &str,
//     from_address: Address,
//     nonce: TxNonce
// ) -> Result<bool> {
//     let provider = ProviderBuilder::new()
//         .connect_http(rpc_url.parse()?);

//     let pending_block = provider
//         .get_block_by_number(alloy::eips::BlockNumberOrTag::Pending)
//         .await
//         .context("helpers: failed to fetch pending block");
// }