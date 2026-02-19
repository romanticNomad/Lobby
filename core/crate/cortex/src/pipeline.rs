use crate::{
    config::RetryConfig,
    error::CortexError,
    pool::ShardPool,
    retry::retry_with_backoff,
    state::{PipelineStatus, StatusRegistry},
};
use kernel::{
    traits::{Broadcaster, IntentRelay, NonceManager, Signer, Validator},
    types::{ClientConfig, Eip1559Transaction, ExecutionId},
};
use std::sync::Arc;
use tracing::Instrument;

// ============================================================
// context

/// all the entities that pipeline would need
/// wrapped in a cheap-to-clone struct
#[derive(Clone)]
pub(crate) struct PipelineContext {
    // request identity
    pub execution_id: ExecutionId,
    pub client_config: ClientConfig,
    pub txn: Eip1559Transaction,

    // actor handles
    pub broadcast_handle: Arc<ShardPool<dyn Broadcaster>>,
    pub nonce_handle: Arc<ShardPool<dyn NonceManager>>,
    pub relayhost_handle: Arc<dyn IntentRelay>,
    pub sign_handle: Arc<ShardPool<dyn Signer>>,
    pub validator_handle: Arc<dyn Validator>,

    // retry
    pub retry_config: RetryConfig,

    // tracing
    pub status: Arc<StatusRegistry>,
}

// ============================================================
// entry point
/// Run the full transaction pipeline for one execution.
///
/// This function is called inside a `tokio::spawn`'d task and never returns an
/// error to the caller — all failures are recorded in `StatusRegistry` and
/// logged.
///
/// ## Failure semantics
///
/// | Stage       | On hard fail                              |
/// |-------------|-------------------------------------------|
/// | RelayHost   | No nonce reserved → just log and exit     |
/// | Nonce       | No nonce reserved → just log and exit     |
/// | Sign        | Release nonce via `resolve(false)` → exit |
/// | Broadcast   | Release nonce via `resolve(false)` → exit |
/// | Validator   | Release nonce via `resolve(false)` → exit |
pub(crate) async fn run_pipeline(ctx: PipelineContext) {
    // ============================================================
    // sharding keys and tracing span

    let execution_id = ctx.execution_id;
    let from_address = ctx.client_config.from_address;
    let chain_id = ctx.txn.chain_id;

    let span = tracing::info_span!(
        "pipeline",
        %execution_id,
        %chain_id,
        %from_address,
    );

    async move {
        tracing::info!("Pipeline started");

        // ============================================================
        // RelayHost

        let rh_result = retry_with_backoff(&ctx.retry_config, "relay_host", || {
            let rh = Arc::clone(&ctx.relayhost_handle);
            let txn = ctx.txn.clone();
            let cc = ctx.client_config.clone();

            async move { rh.send_transaction(execution_id, txn, cc).await }
        })
        .await;

        if let Err(e) = rh_result {
            let err = CortexError::RelayHost(e);
            record_faliure(&ctx.status, execution_id, &err);
            return;
        }

        ctx.status.set(execution_id, PipelineStatus::Accepted);
        tracing::debug!("relay_host: recorded");

        // ============================================================
        // nonce reserve
    }
    .instrument(span)
    .await;
}

// ============================================================
// helper

fn record_faliure(registry: &StatusRegistry, execution_id: ExecutionId, err: &CortexError) {
    tracing::error!(
        %execution_id,
        stage = err.stage(),
        %err,
        "pipeline hard fail",
    );

    registry.set(
        execution_id,
        PipelineStatus::Failed {
            stage: err.stage().to_owned(),
            reason: err.to_string(),
        },
    );
}

// ===========================================================
