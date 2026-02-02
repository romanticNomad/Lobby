use std::{path::PathBuf, str::FromStr};

use alloy_primitives::Address;
use kernel::traits::PolicyEngine;
use props::policy::JsonPolicyEngine;

#[derive(Debug)]
struct JsonPolicyTest {
    key_id: String,
    pvt_key: String,
}

impl JsonPolicyTest {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test_keys.json");
        let set1 = JsonPolicyEngine::load_file(path.to_str().unwrap());
        let from_address: Address = Address::from_str("0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92")
            .expect("address failed");
        let (key_id, pvt_bytes) = set1.resolve_key(&from_address).expect("resolving failed");

        let pvt_key = alloy_primitives::hex::encode(pvt_bytes);
        Self { key_id, pvt_key }
    }
}

#[test]
fn policy_test() {
    let path_test = JsonPolicyTest::new();
    assert_eq!(path_test.key_id, "1");
    assert_eq!(
        path_test.pvt_key,
        "e74176dc8bcf2e5e6500a8f117a665ed44bcf448206c0cd23cb1228e61a2729c"
    );
}
