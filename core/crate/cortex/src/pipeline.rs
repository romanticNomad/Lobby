use crate::{
    artifacts::config::RetryConfig,
    artifacts::error::CortexError,
    artifacts::pool::{ByAddress, ByChainId, ByExecutionId, ShardPool},
    artifacts::retry::retry_with_backoff,
    state::{PipelineStatus, StatusRegistry},
};
use kernel::{
    traits::{Broadcaster, IntentRelay, NonceManager, Signer, Validator},
    types::{ClientConfig, Eip1559Transaction, ExecutionId, ValidatorOutcome},
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
    pub nonce_pool: Arc<ShardPool<dyn NonceManager>>,
    pub sign_pool: Arc<ShardPool<dyn Signer>>,
    pub broadcast_pool: Arc<ShardPool<dyn Broadcaster>>,

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
        // relay host

        let rh_result = retry_with_backoff(&ctx.retry_config, "relay_host", || {
            let rh = Arc::clone(&ctx.relayhost_handle);
            let txn = ctx.txn.clone();
            let cc = ctx.client_config.clone();

            async move { rh.register_transaction(execution_id, txn, cc).await }
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
            async move { nh.reserve(chain_id, from_address, execution_id).await }
        })
        .await
        {
            Ok(n) => n,
            Err(e) => {
                let err = CortexError::NonceReservation(e);
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

        // getting the sign handle from shard pool (sequenced by execution_id)
        let sign_handle = ctx.sign_pool.get(&ByExecutionId(&execution_id));

        let signed = match retry_with_backoff(&ctx.retry_config, "sign", || {
            let sh = Arc::clone(&sign_handle);
            let t = txn.clone();
            async move { sh.sign(chain_id, from_address, execution_id, t).await }
        })
        .await
        {
            Ok(s) => s,
            Err(e) => {
                // signing error -> hard fail
                release_nonce(&nonce_handle, execution_id, &ctx.retry_config).await;

                let err = CortexError::Sign(e);
                record_faliure(&ctx.status, execution_id, &err);
                return;
            }
        };

        ctx.status.set(execution_id, PipelineStatus::Signed);
        tracing::info!("transaction signed");

        // ============================================================
        // broadcast

        // getting the bradcast handle from shard pool (sequenced by chain_id))
        let broadcast_handle = ctx.broadcast_pool.get(&ByChainId(&chain_id));

        let outcome = match retry_with_backoff(&ctx.retry_config, "broadcast", || {
            let bh = Arc::clone(&broadcast_handle);
            let signed_hash = signed.clone();

            async move {
                bh.broadcast(chain_id, from_address, execution_id, signed_hash)
                    .await
            }
        })
        .await
        {
            Ok(broadcast_outcome) => broadcast_outcome,
            Err(e) => {
                // fatal error -> release nonce
                release_nonce(&nonce_handle, execution_id, &ctx.retry_config).await;

                let err = CortexError::Broadcast(e);
                record_faliure(&ctx.status, execution_id, &err);
                return;
            }
        };

        let tx_hash = outcome.txn_hash;
        ctx.status.set(
            execution_id,
            PipelineStatus::Broadcasted {
                tx_hash: format!("{tx_hash:#x}"),
            },
        );
        tracing::info!(
            %tx_hash,
            "transaction broadcasted"
        );

        // ============================================================
        // validator

        let validation = match retry_with_backoff(&ctx.retry_config, "validator", || {
            let v = Arc::clone(&ctx.validator_handle);
            async move { v.validate(chain_id, execution_id, tx_hash).await }
        })
        .await
        {
            Ok(outcome) => outcome,
            Err(e) => {
                release_nonce(&nonce_handle, execution_id, &ctx.retry_config).await;
                let err = CortexError::NotIncluded(e);
                record_faliure(&ctx.status, execution_id, &err);
                return;
            }
        };

        match validation {
            ValidatorOutcome::Included {
                block_number,
                confirmations,
            } => {
                finalise_nonce(&nonce_handle, execution_id, &ctx.retry_config).await;

                ctx.status.set(
                    execution_id,
                    PipelineStatus::Confirmed {
                        tx_hash: format!("{tx_hash:#x}"),
                    },
                );
                tracing::info!(
                    %tx_hash,
                    block_number,
                    confirmations,
                    "transaction confirmed on-chain"
                );
            }
            ValidatorOutcome::NotIncluded => {
                tracing::warn!(%tx_hash, "transaction not included (reorg or eviction)");
                release_nonce(&nonce_handle, execution_id, &ctx.retry_config).await;

                ctx.status.set(
                    execution_id,
                    PipelineStatus::Failed {
                        stage: "validator".to_owned(),
                        reason: format!("tx {tx_hash:#x} not included on-chain"),
                    },
                );
            }
        }

        // ============================================================

        tracing::info!("pipeline completed");
    }
    .instrument(span)
    .await;
}

// ============================================================
// helper

/// Release (invalidate) a reserved nonce so it can be re-used.
///
/// Called on any hard-fail *after* a nonce has been reserved.
/// Retries are attempted because a transient DB error here would leave a
/// dangling 'reserved' nonce; the 5-minute lease is the last-resort safety net.
async fn release_nonce(
    handle: &Arc<dyn NonceManager>,
    execution_id: ExecutionId,
    retry_config: &RetryConfig,
) {
    let resolve_result = retry_with_backoff(&retry_config, "nonce_release", || {
        let nh = Arc::clone(handle);
        async move { nh.resolve(execution_id, false).await }
    })
    .await;

    if let Err(e) = resolve_result {
        // not fatal -> lease will expire in 5 minutes
        tracing::error!(
            %execution_id,
            %e,
            "nonce release failed after retries, lease will expire in 5 min"
        );
    } else {
        tracing::debug!(%execution_id, "nonce release");
    }
}

/// Finalize (consume) a reserved nonce after on-chain inclusion.
async fn finalise_nonce(
    handle: &Arc<dyn NonceManager>,
    execution_id: ExecutionId,
    retry_config: &RetryConfig,
) {
    let resolve_result = retry_with_backoff(&retry_config, "nonce_finalise", || {
        let nh = Arc::clone(handle);
        async move { nh.resolve(execution_id, true).await }
    })
    .await;

    if let Err(e) = resolve_result {
        // not fatal -> dangling nonce lease expires in 5 min
        tracing::error!(
            %execution_id,
            %e,
            "nonce finalising failed -> state remains 'reserved' lease will expire in 5 minutes"
        );
    } else {
        tracing::debug!(%execution_id, "nonce finalised");
    }
}

/// helper to record fatal errors.
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
