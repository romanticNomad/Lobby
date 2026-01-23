use tokio::sync::{mpsc, oneshot};
use alloy_primitives::{Address};
use kernel::traits::NonceManager;
use kernel::types::{ChainId, ExecutionError, ExecutionId, TxNonce};


// Commands to send over the channel.
pub enum NonceCommand {
    Reserve {
        chain_id: ChainId,
        from: Address,
        id: ExecutionId,
        reply: oneshot::Sender<Result<TxNonce, ExecutionError>>,
    },
    Resolve {
        chain_id: ChainId,
        from: Address,
        outcome: bool,
        reply: oneshot::Sender<Result<(), ExecutionError>>,
    }
}
