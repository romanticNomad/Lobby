use std::{path::PathBuf, str::FromStr};

use alloy::primitives::{Address, hex};
use kernel::traits::PolicyEngine;
use props::policy::JsonPolicyEngine;

#[derive(Debug)]
struct JsonPolicyTest {
    pvt_key: String,
}

impl JsonPolicyTest {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test_keys.json");
        let set1 = JsonPolicyEngine::load_file(path.to_str().unwrap());
        let from_address: Address = Address::from_str("0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92")
            .expect("address failed");
        let pvt_bytes = set1.resolve_key(&from_address).expect("resolving failed");

        let pvt_key = hex::encode(pvt_bytes);
        Self { pvt_key }
    }
}

#[test]
fn policy_test() {
    let path_test = JsonPolicyTest::new();
    assert_eq!(
        path_test.pvt_key,
        "e74176dc8bcf2e5e6500a8f117a665ed44bcf448206c0cd23cb1228e61a2729c"
    );
}
