use alloy::primitives::Address;
use dashmap::DashMap;
use primitives::{traits::PolicyEngine, types::LocalError};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::{fs::File, io::BufReader, path::PathBuf};

// ============================================================
// Policy data structures

/// ## Lobby `JSON-ACCOUNT` format
/// The evm-account details need to be stored in this format.
#[derive(Deserialize, Debug)]
struct PolicyAccount {
    pvt_key: String,
    pub_key: String,
    address: String,
}

/// ## Adress Mapping for Lobby
/// ### features
///
/// * (evm_address, pvt_keys) stored as key-value pairs in Dashmap to allow multi-thread access.
/// * `Arc` backed pvt-keys, to prevent heap cloning.
#[derive(Debug)]
pub struct JsonPolicyEngine {
    keys: DashMap<Address, Arc<String>>,
}

impl JsonPolicyEngine {
    pub fn load_file(path: &str) -> Self {
        let file = File::open(path).unwrap_or_else(|e| panic!("Unable to open file {}. {}", path, e));
        let reader = BufReader::new(file);

        let raw: HashMap<String, PolicyAccount> =
            serde_json::from_reader(reader).unwrap_or_else(|e| panic!("Unable to read file {}. {}", path, e));
        let keys = DashMap::new();

        for account in raw.values() {
            let address: Address = account.address.parse().expect("address invalid");

            if keys.contains_key(&address) {
                panic!("duplicate address found in policy")
            }

            keys.insert(address, Arc::new(account.pvt_key.clone()));
        }

        Self { keys }
    }
}

// ============================================================
// Policy implimentation

impl PolicyEngine for JsonPolicyEngine {
    fn resolve_key(&self, from: &Address) -> Result<[u8; 32], LocalError> {
        let pvt_string = self
            .keys
            .get(from)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| {
                LocalError::Internal(format!("Policy violation: no Key detected for: {}", from))
            })?;

        let pvt_str = pvt_string
            .strip_prefix("0x")
            .ok_or(LocalError::Internal("Unable to parse Pvt Keys".into()))?;
        let pvt_bytes: [u8; 32] = hex::decode(pvt_str)
            .map_err(|e| LocalError::Invariant(e.to_string()))?
            .try_into()
            .map_err(|e| LocalError::Invariant(format!("Invalid pvt key length: {:?}", e)))?;

        Ok(pvt_bytes)
    }
}

// ============================================================
// helper function

/// return the number of keys / evm accounts in custody of lobby
pub fn export_custody_key_count() -> usize {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test_keys.json");
    let file = JsonPolicyEngine::load_file(path.to_str().unwrap());
    file.keys.len()
}

// ============================================================
