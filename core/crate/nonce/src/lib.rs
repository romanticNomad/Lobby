// use alloy_primitives::Address;
// use kernel::traits::NonceManager;
// use kernel::types::{ChainId, ExecutionError, ExecutionId, TxNonce};

// #[derive(Clone, Copy, Debug, PartialEq, Eq)]
// pub enum NonceStatus {
//     Reserved,
//     Broadcasted,
//     Confirmed,
//     Dropped,
// }

// #[derive(Debug)]
// pub struct LobbyNonce {
//     pub chain_id: ChainId,
//     pub from: Address,
//     pub nonce: TxNonce,
//     pub status: NonceStatus,
//     pub execution_id: ExecutionId,
// }

// impl NonceManager for LobbyNonce {
//     async fn reserve(&self, chain_id: ChainId, from: Address) -> Result<TxNonce, ExecutionError> {}

//     async fn resolve(
//         &self,
//         chain_id: ChainId,
//         from: Address,
//         id: ExecutionId,
//         outcome: bool,
//     ) -> Result<(), ExecutionError> {
//     }
// }
