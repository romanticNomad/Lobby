// use crate::{config::RetryConfig, pool::ShardPool, state::StatusRegistry};
// use kernel::{
//     traits::{Broadcaster, IntentRelay, NonceManager, Signer, Validator},
//     types::{ClientConfig, Eip1559Transaction, ExecutionId},
// };
// use std::sync::Arc;

// // ============================================================
// // context

// /// all the entities that pipeline would need
// /// wrapped in a cheap-to-clone struct
// #[derive(Clone)]
// pub(crate) struct PipelineContext {
//     // request identity
//     pub execution_id: ExecutionId,
//     pub client_config: ClientConfig,
//     pub txn: Eip1559Transaction,

//     // actor handles
//     pub broadcast_handle: Arc<ShardPool<dyn Broadcaster>>,
//     pub nonce_handle: Arc<ShardPool<dyn NonceManager>>,
//     pub relayhost_handle: Arc<dyn IntentRelay>,
//     pub sign_handle: Arc<ShardPool<dyn Signer>>,
//     pub validator_handle: Arc<dyn Validator>,

//     // retry
//     pub retry_config: RetryConfig,

//     // tracing
//     pub status: Arc<StatusRegistry>,
// }

// // ============================================================
// // entry point
// /// Run the full transaction pipeline for one execution.
// ///
// /// This function is called inside a `tokio::spawn`'d task and never returns an
// /// error to the caller — all failures are recorded in `StatusRegistry` and
// /// logged.
// ///
// /// ## Failure semantics
// ///
// /// | Stage        | On hard fail                              |
// /// |-------------|-------------------------------------------|
// /// | RelayHost   | No nonce reserved → just log and exit     |
// /// | Nonce       | No nonce reserved → just log and exit     |
// /// | Sign        | Release nonce via `resolve(false)` → exit |
// /// | Broadcast   | Release nonce via `resolve(false)` → exit |
// /// | Validator   | Release nonce via `resolve(false)` → exit |
