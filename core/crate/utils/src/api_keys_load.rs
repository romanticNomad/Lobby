use std::env;

use alloy::primitives::Address;
use dashmap::DashMap;
use kernel::types::{ApiKey, ClientConfig};
use uuid::Uuid;

// ============================================================
// function for loading api keys from the env (for dev testing only)

pub fn load_api_key_from_env() -> Result<DashMap<ApiKey, ClientConfig>, Box<dyn std::error::Error>>
{
    let api_keys = DashMap::new();

    // Format: LOBBY_API_KEY_<N>=<api_key>:<client_id>:<from_address>
    for (key, value) in env::vars() {
        if let Some(suffix) = key.strip_prefix("LOBBY_API_KEY_".to_string()) {
            let parts: Vec<&str> = value.split(":").collect();
            if parts.len() != 3 {
                return Err(format!(
                    "Invalid API key format for {}: expected <api_key>:<client_id>:<from_address>",
                    key
                )
                .into());
            }

            let apki_key = parts[0].to_string();
            let client_id =
                Uuid::parse_str(parts[1]).map_err(|_| format!("invalid client id: {}", key))?;
            let from_address = Address::from(parts[2].to_string());
        }
    }

    Ok(())
}

// ============================================================
