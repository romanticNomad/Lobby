use crate::{
    artifacts::config::RetryConfig,
    artifacts::error::CortexError,
    artifacts::pool::{ByAddress, ByChainId, ByExecutionId, ShardPool},
    artifacts::retry::retry_with_backoff,
    state::{PipelineStatus, StatusRegistry},
};
use kernel::{
    traits::{Broadcaster, IntentRelay, NonceManager, Signer, Validator},
    types::{BroadcastError, ClientConfig, Eip1559Transaction, ExecutionId, ValidatorOutcome},
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
    pub status: StatusRegistry,
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
/// |             | NonceTooLow → sync and retry once         |
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

        let mut signed = match retry_with_backoff(&ctx.retry_config, "sign", || {
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
        // broadcast (with nonce mismatch retry)

        // getting the broadcast handle from shard pool (sequenced by chain_id))
        let broadcast_handle = ctx.broadcast_pool.get(&ByChainId(&chain_id));

        // Guard against infinite retry loop
        let mut nonce_retry_attempted = false;

        let outcome = loop {
            match retry_with_backoff(&ctx.retry_config, "broadcast", || {
                let bh = Arc::clone(&broadcast_handle);
                let signed_clone = signed.clone();

                async move {
                    bh.broadcast(chain_id, from_address, execution_id, signed_clone)
                        .await
                }
            })
            .await
            {
                Ok(broadcast_outcome) => break broadcast_outcome,

                // ============================================================
                // NONCE TOO LOW - Attempt one-time recovery
                Err(BroadcastError::NonceTooLow {
                    nonce_on_chain,
                    attempted_nonce,
                }) => {
                    if nonce_retry_attempted {
                        // Already retried once, give up to prevent infinite loop
                        tracing::error!(
                            %execution_id,
                            %nonce_on_chain,
                            %attempted_nonce,
                            "nonce mismatch retry failed after sync, aborting pipeline"
                        );

                        release_nonce(&nonce_handle, execution_id, &ctx.retry_config).await;

                        let err = CortexError::Broadcast(BroadcastError::NonceTooLow {
                            nonce_on_chain,
                            attempted_nonce,
                        });
                        record_faliure(&ctx.status, execution_id, &err);
                        return;
                    }

                    nonce_retry_attempted = true;

                    tracing::warn!(
                        %execution_id,
                        %attempted_nonce,
                        %nonce_on_chain,
                        "nonce too low detected - initiating sync and retry"
                    );

                    // Update status to indicate nonce sync is happening
                    ctx.status.set(
                        execution_id,
                        PipelineStatus::SyncNonce {
                            nonce_on_chain,
                            attempted_nonce,
                        },
                    );

                    // ============================================================
                    // Step 1: Release the incorrect nonce and revert sign status.

                    release_nonce(&nonce_handle, execution_id, &ctx.retry_config).await;
                    revert_sign(&sign_handle, execution_id, &ctx.retry_config).await;
                    tracing::debug!(%execution_id, "released incorrect nonce");

                    // ============================================================
                    // Step 2: Sync with on-chain state and reserve correct nonce

                    let corrected_nonce = match retry_with_backoff(
                        &ctx.retry_config,
                        "nonce_sync_and_reserve",
                        || {
                            let nh = Arc::clone(&nonce_handle);
                            async move {
                                nh.sync_and_reserve(
                                    chain_id,
                                    from_address,
                                    execution_id,
                                    nonce_on_chain,
                                )
                                .await
                            }
                        },
                    )
                    .await
                    {
                        Ok(n) => n,
                        Err(e) => {
                            tracing::error!(
                                %execution_id,
                                error = %e,
                                "failed to sync and reserve nonce after retry"
                            );
                            let err = CortexError::NonceReservation(e);
                            record_faliure(&ctx.status, execution_id, &err);
                            return;
                        }
                    };

                    tracing::info!(
                        %execution_id,
                        %corrected_nonce,
                        "nonce synced and reserved after on-chain mismatch"
                    );

                    ctx.status.set(execution_id, PipelineStatus::NonceReserved);

                    // ============================================================
                    // Step 3: Update transaction with corrected nonce

                    txn.nonce = corrected_nonce;

                    // ============================================================
                    // Step 4: Re-sign transaction with corrected nonce

                    signed = match retry_with_backoff(&ctx.retry_config, "sign_retry", || {
                        let sh = Arc::clone(&sign_handle);
                        let t = txn.clone();
                        async move { sh.sign(chain_id, from_address, execution_id, t).await }
                    })
                    .await
                    {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::error!(
                                %execution_id,
                                error = %e,
                                "re-signing failed after nonce sync"
                            );
                            release_nonce(&nonce_handle, execution_id, &ctx.retry_config).await;
                            let err = CortexError::Sign(e);
                            record_faliure(&ctx.status, execution_id, &err);
                            return;
                        }
                    };

                    tracing::info!(
                        %execution_id,
                        "transaction re-signed with corrected nonce"
                    );
                    ctx.status.set(execution_id, PipelineStatus::Signed);

                    // ============================================================
                    // Step 5: Loop will retry broadcast with new signed transaction

                    tracing::info!(%execution_id, "retrying broadcast with synced nonce");
                    continue;
                }

                // ============================================================
                // OTHER BROADCAST ERRORS - Hard fail
                Err(e) => {
                    release_nonce(&nonce_handle, execution_id, &ctx.retry_config).await;
                    let err = CortexError::Broadcast(e);
                    record_faliure(&ctx.status, execution_id, &err);
                    return;
                }
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
        let v = Arc::clone(&ctx.validator_handle);
        let validation =
            match { async move { v.validate(chain_id, execution_id, tx_hash).await } }.await {
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
                    %block_number,
                    %confirmations,
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
        tracing::debug!(%execution_id, "nonce released");
    }
}

/// Revert sign status from 'signed' to 'failed'
/// for handeling errors require resigning.
/// **example**: lobby DB and on-chain nonce mismatch
async fn revert_sign(
    handle: &Arc<dyn Signer>,
    execution_id: ExecutionId,
    retry_config: &RetryConfig,
) {
    let revert_result = retry_with_backoff(&retry_config, "sign_revert", || {
        let sh = Arc::clone(handle);
        async move { sh.revert(execution_id).await }
    })
    .await;

    if let Err(e) = revert_result {
        // not fatal -> lease will expire in 5 minutes
        tracing::error!(
            %execution_id,
            %e,
            "sign state revertion failed"
        );
    } else {
        tracing::debug!("sign state set to failed")
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
