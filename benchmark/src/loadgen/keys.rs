use dashmap::DashMap;
use hex::encode;
use k256::ecdsa::SigningKey;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha3::{Digest, Keccak256};
use std::{collections::HashMap, fs, path::Path};
use uuid::Uuid;

// test-keys.json elements
// ============================================================

/// SerDe format for private key, public key, and public address.
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
// test_keys.json generater

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
pub fn tkgen(sample_size: u64) -> Result<(), Box<dyn std::error::Error>> {
    let mut test_keys_map: HashMap<String, EvmKeyExport> = HashMap::new();
    for i in 0..=sample_size {
        let (pvt_key, pub_key, address) = kgen();
        let account = EvmKeyExport::new(pvt_key, pub_key, address);
        let entry = format!("account{}", i);

        test_keys_map.insert(entry, account);
    }

    let path = "test_keys.json";
    let test_keys_json = serde_json::to_string_pretty(&test_keys_map)?;

    fs::write(path, test_keys_json)?;
    tracing::info!("{} account created in test_keys.json", sample_size);
    Ok(())
}

// ============================================================
// keys generator

/// Helper function that generates EVM compatibel keys
///
/// Generates:
/// * `pvt_key`
/// * `pub_key`
/// * `evm_address`
fn kgen() -> (String, String, String) {
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

// lobby-api keys elements
// ============================================================

/// Lobby-Api keys stack, to be set up in the environment along with docker urls
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
pub fn get_apistack(fileplath: &Path) -> Result<ApiStack, Box<dyn std::error::Error>> {
    let api_stack: ApiStack = DashMap::new();

    let file_contents = fs::read_to_string(fileplath)?;
    let parsed_content: Value = serde_json::from_str(&file_contents)?;

    let object_map = parsed_content
        .as_object()
        .ok_or("test_keys.json must contain a top-level JSON object")?;
    let extracted_accounts: Vec<(&String, &Value)> = object_map.iter().collect();

    for (account_name, keys) in extracted_accounts {
        let account_num = account_name
            .strip_prefix("account")
            .and_then(|num_str| num_str.parse::<u64>().ok())
            .ok_or("invalid account naming")?;

        let from_address = keys
            .get("address")
            .and_then(|address| address.as_str())
            .ok_or("invalid account address")?
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

// ============================================================

// Helper functions
// ============================================================

/// Collects addresses from the `ApiStack`.
///
/// return a `Vec<String>`.
pub fn get_addresses(api_stack: &ApiStack) -> Vec<String> {
    let address_stack: Vec<String> = api_stack
        .iter()
        .map(|element| {
            let api_key_split: Vec<&str> = element.value().split(":").collect();
            if !api_key_split.len() == 3 {
                panic!("invalid address keys");
            } else {
                let from_address = api_key_split[2];
                from_address.to_string()
            }
        })
        .collect();

    address_stack
}

// ============================================================
