use crate::infra::ApiStack;
use alloy::primitives::Address;
use std::collections::HashMap;

// ============================================================

/// Registry to store the dummy `evm-key`(s) and corresponding `api-key`(s)
pub struct KeyRegistry {
    /// Hasmap to store the (public, private) keys in a (key, value) pair
    evm_key: HashMap<Address, [u8; 32]>,

    /// ApiStack to store the lobby `Api-Key` generated for a given `from_address`
    api_key: ApiStack,
}

// ============================================================
