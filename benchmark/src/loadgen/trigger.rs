use crate::loadgen::keys::{ApiStack, get_addresses};

// ============================================================

/// Placeholder for collection of addresses derived from the `ApiStack`
pub struct Addresses {
    collection: Vec<String>,
}

impl Addresses {
    pub fn new(api_stack: &ApiStack) -> Self {
        let collection = get_addresses(api_stack);

        Self { collection }
    }
}

// ============================================================
