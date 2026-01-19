use alloy_primitives::{Bytes, B256};

#[derive(Clone, Debug)]
pub struct RawTransaction {
    pub rlp: Bytes,
}

#[derive(Clone, Debug)]
pub struct SignedTransaction {
    pub rlp: Bytes,
}

pub type TxHash = B256;
