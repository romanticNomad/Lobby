use crate::{
    config::RetryConfig,
    error::CortexError,
    pool::{ByAddress, ByExecutionId, ShardPool},
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

/// all the entities that pipeline would need,
/// wrapped in a cheap-to-clone struct
#[derive(Clone)]
pub(crate) struct PipelineContext {
    // request identity
    pub execution_id: ExecutionId,
    pub client_config: ClientConfig,
    pub txn: Eip1559Transaction,

    // actor handles
    pub relayhost_handle: Arc<dyn IntentRelay>,
    pub validator_handle: Arc<dyn Validator>,

    // actor pools
    pub broadcast_pool: Arc<ShardPool<dyn Broadcaster>>,
    pub nonce_pool: Arc<ShardPool<dyn NonceManager>>,
    pub sign_pool: Arc<ShardPool<dyn Signer>>,

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

            async move {
                rh.send_transaction(execution_id, txn, cc).await
            }
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

        // geting the nonce handle from shard pool (squenced by from_address)
        let nonce_handle = ctx.nonce_pool.get(&ByAddress(&from_address));

        let nonce = match retry_with_backoff(&ctx.retry_config, "nonce_reserve", || {
            let nh = Arc::clone(&nonce_handle);
            async move {
                nh.reserve(chain_id, from_address, execution_id).await
            }
        })
        .await
        {
            Ok(n) => n,
            Err(e) => {
                let err  = CortexError::NonceReservation(e);
                record_faliure(&ctx.status, execution_id, &err);
                return;
            }
        };

        ctx.status.set(execution_id, PipelineStatus::NonceReserved);
        tracing::info!(%nonce, "nonce reserved");

        // updating the nonce onto txn payload
        let mut txn = ctx.txn;
        txn.nonce = nonce;

        // ============================================================
        // signing
        
        //getting the sign handle from shard pool (sequenced by execution_id)
        let sign_handle = ctx.sign_pool.get(&ByExecutionId(&execution_id));

        let signed = match retry_with_backoff(&ctx.retry_config, "signing", || {
            let sh = Arc::clone(&sign_handle);
            let t = txn.clone();
            async move {
                sh.sign(chain_id, from_address, execution_id, t).await
            }
        })
        .await
        {
            Ok(tx_hash) => tx_hash,
            Err(e) => {
                let err = CortexError::Sign(e);
                record_faliure(&ctx.status, execution_id, &err);
                return;
            }
        };

        ctx.status.set(execution_id, PipelineStatus::Signed);
        tracing::info!("transaction signed");

        // ============================================================
        // broadcast

        

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
