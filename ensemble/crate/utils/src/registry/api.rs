use alloy::primitives::Address;
use dashmap::DashMap;
use kernel::types::{ApiRegistry, ClientConfig};
use std::{env, str::FromStr, sync::Arc};
use thiserror::Error;
use uuid::Uuid;

// ============================================================

#[derive(Debug, Error)]
pub enum EnvApiKeyError {
    #[error("invalid client id: {0}")]
    InvalidClientId(String),
    #[error("invalid from_address {0}")]
    InvalidFromAddress(String),
    #[error("Invalid API key format for {0}: expected <api_token>:<client_id>:<from_address>")]
    InvalidApiKey(String),
    #[error("could not find api_keys of valid format in env")]
    ApiKeyUnavailable,
}

// ============================================================
// function for loading api keys from the env (for dev testing only)

pub fn load_api_key_from_env() -> Result<ApiRegistry, EnvApiKeyError> {
    let api_keys = DashMap::new();

    // Format: LOBBY_API_KEY_<N>=<api_token>:<client_id>:<from_address>
    for (key, value) in env::vars() {
        if let Some(suffix) = key.strip_prefix("LOBBY_API_KEY_") {
            let parts: Vec<&str> = value.split(":").collect();
            if parts.len() != 3 {
                return Err(EnvApiKeyError::InvalidApiKey(key));
            }

            let api_token = parts[0].to_string();
            let client_id = Uuid::parse_str(parts[1])
                .map_err(|_| EnvApiKeyError::InvalidClientId(format!("{}", &key)))?;
            let from_address = Address::from_str(parts[2])
                .map_err(|_| EnvApiKeyError::InvalidFromAddress(format!("{}", &key)))?;

            let client_config = ClientConfig {
                client_id,
                from_address,
            };

            api_keys.insert(api_token, client_config);

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

    Ok(Arc::new(api_keys))
}

// ============================================================
