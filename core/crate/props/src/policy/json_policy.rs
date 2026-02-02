use std::{collections::HashMap, fs::File, io::BufReader};

use alloy_primitives::Address;
use kernel::{traits::PolicyEngine, types::ExecutionError};
use serde::Deserialize;

// ============================================================

#[derive(Deserialize, Debug)]
struct PolicyAccount {
    key_id: String,
    pvt_key: String,
    pub_key: String,
    address: String,
}

// ============================================================

#[derive(Debug)]
pub struct JsonPolicyEngine {
    index: HashMap<Address, (String, String)>,
}

impl JsonPolicyEngine {
    pub fn load_file(path: &str) -> Self {
        let file = File::open(path).expect("policy file path invalid");
        let reader = BufReader::new(file);

        let raw: HashMap<String, PolicyAccount> =
            serde_json::from_reader(reader).expect("Policy file invalid");
        let mut index = HashMap::new();

        for account in raw.values() {
            let address: Address = account.address.parse().expect("address invalid");

            if index.contains_key(&address) {
                panic!("duplicate address found in policy")
            }

            index.insert(address, (account.key_id.clone(), account.pvt_key.clone()));
        }

        Self { index }
    }
}

// ============================================================

impl PolicyEngine for JsonPolicyEngine {
    fn resolve_key(&self, from: &Address) -> Result<(String, [u8; 32]), ExecutionError> {
        let (key_id, pvt_string) = self.index.get(from).ok_or_else(|| {
            ExecutionError::Internal(format!("Policy violation: no Key detected for: {}", from))
        })?;

        let pvt_str = pvt_string.strip_prefix("0x").unwrap_or(pvt_string);
        let pvt_bytes: [u8; 32] = hex::decode(pvt_str)
            .map_err(|e| ExecutionError::Invariant(e.to_string()))?
            .try_into()
            .map_err(|e| ExecutionError::Invariant(format!("Invalid pvt key length: {:?}", e)))?;

        Ok((key_id.clone(), pvt_bytes))
    }
}

// ============================================================
