use crate::types::tx_artifacts::TxHash;

#[derive(Clone, Debug)]
pub enum BroadcastOutcome {
    Submitted { tx_hash: TxHash },
    Rejected { reason: String }, // definitively not submitted
    Unknown,                     // may have been submitted
}
