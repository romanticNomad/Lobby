#[derive(Clone, Debug)]
pub enum ExecutionError {
    InvalidIntent,
    NonceFailure,
    EncodingFailure,
    SigningFailure,
    BroadcastFailure,
    ValidationFailure,
    Internal(String),
    Rejected(String),
}
