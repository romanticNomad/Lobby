use kernel::types::{ExecutionError, RawTransaction, SignedTransaction};
use tokio::sync::{mpsc, oneshot};

pub struct SignCommand {
    txn: RawTransaction,
    reply: oneshot::Sender<Result<SignedTransaction, ExecutionError>>,
}

pub struct SignRelay {
    tx: mpsc::Sender<SignCommand>,
}
