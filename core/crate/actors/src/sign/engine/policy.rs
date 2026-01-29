use std::{collections::HashMap, fs::File, io::BufReader};

use serde::Deserialize;
use alloy_primitives::Address;
// use kernel::traits::PolicyEngine;


#[derive(Deserialize, Debug)]
struct PolicyAccount {
    key_id: String,
    pvt_key: String,
    pub_key: String,
    address: String,
}

pub struct JsonPolicyEngine {
    index: HashMap<Address, (String, String)>,
}

impl JsonPolicyEngine {
    pub fn load_file(path: &str) -> Self {
        let file = File::open(path)
            .expect("policy file path invalid");
        let reader = BufReader::new(file);

        let raw: HashMap<String, PolicyAccount> = serde_json::from_reader(reader).expect("Policy file invalid");
        let mut index = HashMap::new();

        for account in raw.values() {
            let address: Address = account.address.parse().expect("address invalid");

            if index.contains_key(&address) {
                panic!("duplicate address found in policy")
            }

            index.insert(
                address,
                (account.key_id.clone(), account.pvt_key.clone())
            );
        }

        Self { index }
    }
}
