use alloy_primitives::{B256, Bytes, U256};

pub type TxHash = B256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxNonce(pub U256);

#[derive(Clone, Debug)]
pub struct RawTransaction {
    pub rlp: Bytes,
}

#[derive(Clone, Debug)]
pub struct SignedTransaction {
    pub rlp: Bytes,
}

#[derive(Clone, Debug)]
pub enum BroadcastOutcome {
    Submitted { tx_hash: TxHash },
    Rejected { reason: String }, // definitively not submitted
    Unknown,                     // may have been submitted
}
