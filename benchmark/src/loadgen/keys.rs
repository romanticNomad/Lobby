use alloy::primitives::Address;
use anyhow::{Context, Result};
use dashmap::DashMap;
use hex::encode;
use k256::ecdsa::SigningKey;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha3::{Digest, Keccak256};
use std::str::FromStr;
use std::{collections::HashMap, fs, path::Path};
use uuid::Uuid;

// test-keys.json elements
// ============================================================

/// Serde format for private key, public key, and public address.
#[derive(Serialize, Deserialize)]
pub struct EvmKeyExport {
    pvt_key: String,
    pub_key: String,
    address: String,
}

impl EvmKeyExport {
    pub fn new(pvt_key: String, pub_key: String, address: String) -> Self {
        Self {
            pvt_key,
            pub_key,
            address,
        }
    }
}

// ============================================================
// test_keys.json generator

/// Generates the test_keys.json file for benchmark
///
/// File format:
/// ```json
/// {
///   "account1": {
///     "pvt_key": "0xeca7c841791...",
///     "pub_key": "0xffdefb6deb0...",
///     "address": "0x430b3af2c71..."
///    },
/// }
/// ```
pub fn write_test_keys_json(sample_size: u64) -> Result<()> {
    let mut test_keys_map: HashMap<String, EvmKeyExport> = HashMap::new();
    for i in 1..=sample_size {
        let (pvt_key, pub_key, address) = test_key_gen();
        let account = EvmKeyExport::new(pvt_key, pub_key, address);
        let entry = format!("account{}", i);

        test_keys_map.insert(entry, account);
    }

    let path = "benchmark/test_keys.json";
    let test_keys_json = serde_json::to_string_pretty(&test_keys_map)?;

    fs::write(path, test_keys_json)?;
    tracing::info!("{} account created in test_keys.json", sample_size);
    Ok(())
}

// ============================================================
// api-keys generator

/// Helper function that generates EVM compatible keys
///
/// Generates:
/// * `pvt_key`
/// * `pub_key`
/// * `evm_address`
fn test_key_gen() -> (String, String, String) {
    // pvt_key
    let signing_key = SigningKey::random(&mut OsRng);
    let verify_key = signing_key.verifying_key();

    let private_key_bytes = signing_key.to_bytes();
    let private_key_hex = format!("0x{}", encode(private_key_bytes));

    // pub_key
    let pub_key_encode = verify_key.to_encoded_point(false);
    let pub_key_bytes = pub_key_encode.as_bytes();
    let pub_key_hex = format!("0x{}", encode(pub_key_bytes));

    // evm_address
    let keccak_hash = Keccak256::digest(&pub_key_bytes[1..]);
    let evm_address_hex = format!("0x{}", encode(&keccak_hash[12..]));

    (private_key_hex, pub_key_hex, evm_address_hex)
}

// ============================================================
// lobby-api keys elements

/// Lobby-Api keys stack, to be set up in the environment along with docker URLs
///
/// API Key format:
///
/// `LOBBY_API_KEY_<N>=<api_token>:<client_id>:<from_address>`
pub type ApiStack = DashMap<u64, String>;

/// Generates API keys for the provided accounts.json file.
///
/// File format required:
/// ```json
/// {
///   "account1": {
///     "pvt_key": "0xeca7c841791...",
///     "pub_key": "0xffdefb6deb0...",
///     "address": "0x430b3af2c71..."
///    },
/// }
/// ```
pub fn build_apistack(filepath: &Path) -> Result<ApiStack> {
    let api_stack: ApiStack = DashMap::new();

    let file_contents = fs::read_to_string(filepath)?;
    let parsed_content: Value = serde_json::from_str(&file_contents)?;

    let object_map = parsed_content
        .as_object()
        .context("test_keys.json must contain a top-level JSON object")?;
    let extracted_accounts: Vec<(&String, &Value)> = object_map.iter().collect();

    for (account_name, keys) in extracted_accounts {
        let account_num = account_name
            .strip_prefix("account")
            .and_then(|num_str| num_str.parse::<u64>().ok())
            .context("invalid account naming")?;

        let from_address = keys
            .get("address")
            .and_then(|address| address.as_str())
            .context("invalid account address")?
            .to_string();

        let client_id = Uuid::new_v4();
        let api_token = {
            let token_string = Uuid::new_v4().simple().to_string();
            format!("lobby_live_{}", &token_string[..9])
        };

        let lobby_api_key = format!("{}:{}:{}", api_token, client_id, from_address);
        api_stack.insert(account_num, lobby_api_key);
    }

    Ok(api_stack)
}

pub fn get_addresses(api_stack: &ApiStack) -> Result<Vec<Address>> {
    let addresses: Vec<Address> = api_stack
        .into_iter()
        .map(|element| {
            let api_key = element.value();
            let api_key_elements: Vec<&str> = api_key.split(':').collect();
            let address_str = api_key_elements[2];
            let evm_address = Address::from_str(address_str).unwrap();
            evm_address
        })
        .collect();

    Ok(addresses)
}

// ============================================================
// unit test

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn unit_test_keys_gen() -> Result<()> {
        // generate 10 sample keys
        let generate_keys = write_test_keys_json(2);
        assert!(generate_keys.is_ok());

        // test if the keys generated produce the correct api_keys.
        let path = Path::new("test_keys.json");
        let api_stack = build_apistack(path).expect("failed to build API stack");
        let api_stack_payload = serde_json::to_string_pretty(&api_stack)?;
        fs::write("test_api_keys.json", api_stack_payload)?;

        Ok(())
    }
}

// ============================================================
