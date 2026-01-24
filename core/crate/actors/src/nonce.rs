// use alloy_primitives::Address;
// use async_trait::async_trait;
// use kernel::{
//     traits::NonceManager,
//     types::{ChainId, ExecutionError, ExecutionId, TxNonce},
// };
// use tokio::sync::{mpsc, oneshot};

// // =========================================================
// // Commands to send over the channel.

// pub enum NonceCommand {
//     Reserve {
//         chain_id: ChainId,
//         from: Address,
//         id: ExecutionId,
//         reply: oneshot::Sender<Result<TxNonce, ExecutionError>>,
//     },
//     Resolve {
//         chain_id: ChainId,
//         from: Address,
//         id: ExecutionId,
//         outcome: bool,
//         reply: oneshot::Sender<Result<(), ExecutionError>>,
//     },
// }

// // =========================================================
// // Entry point for Lobby into nonce actor.

// #[derive(Clone)]
// pub struct NonceChannel {
//     tx: mpsc::Sender<NonceCommand>,
// }

// impl NonceChannel {
//     pub fn new(tx: mpsc::Sender<NonceCommand>) -> Self {
//         Self { tx }
//     }
// }

// // =========================================================
// // implimentation of NonceManager for NonceChannel.

// #[async_trait]
// impl NonceManager for NonceChannel {
//     async fn reserve(nonce_id: TxNonce, from: Address, id: ExecutionId) -> Result<TxNonce, ExecutionError> {

//     }
// }
