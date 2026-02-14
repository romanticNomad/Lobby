use std::{env, str::FromStr};

use alloy::primitives::Address;
use dashmap::DashMap;
use kernel::types::{ApiKey, ClientConfig};
use uuid::Uuid;

// ============================================================

#[derive(Debug)]
pub enum EnvApiKeyError {
    InvalidClientId(String),
    InvalidFromAddress(String),
    InvalidApiKey(String),
    ApiKeyUnavailable,
}

// ============================================================
// function for loading api keys from the env (for dev testing only)

pub fn load_api_key_from_env() -> Result<DashMap<ApiKey, ClientConfig>, EnvApiKeyError> {
    let api_keys = DashMap::new();

    // Format: LOBBY_API_KEY_<N>=<api_key>:<client_id>:<from_address>
    for (key, value) in env::vars() {
        if let Some(suffix) = key.strip_prefix("LOBBY_API_KEY_") {
            let parts:Vec<&str> = value.split(":").collect();
            if parts.len() != 3 {
                return Err(EnvApiKeyError::InvalidApiKey(format!(
                    "Invalid API key format for {}: expected <api_key>:<client_id>:<from_address>",
                    key
                ))
                .into())
            }

            let api_key = parts[0].to_string();
            let client_id = Uuid::parse_str(parts[1])
                .map_err(|_| EnvApiKeyError::InvalidClientId(format!("invalid client id: {}", key)))?;
            let from_address = Address::from_str(parts[2])
                .map_err(|_| EnvApiKeyError::InvalidFromAddress(format!("invalid from_address {}", key)))?;

            let client_config = ClientConfig {
                client_id,
                from_address
            };

            api_keys.insert(api_key, client_config);

            tracing::debug!(
                key = suffix,
                client_id = %client_id,
                from_address = ?from_address,
                "loaded api key"                
            );
        }
    }

    if api_keys.is_empty() {
        return Err(EnvApiKeyError::ApiKeyUnavailable);
    }

    Ok(api_keys)
}

// ============================================================
