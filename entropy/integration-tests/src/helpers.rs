use alloy::{
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
};
use anyhow::{Context, Result};

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
