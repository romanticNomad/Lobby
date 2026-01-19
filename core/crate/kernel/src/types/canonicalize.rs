use alloy_primitives::Bytes;

#[derive(Clone, Debug)]
pub struct RawTransaction {
    pub rlp: Bytes,
}

#[derive(Clone, Debug)]
pub struct SignedTransaction {
    pub rlp: Bytes,
}
