use crate::{
    artifacts::config::RetryConfig,
    artifacts::error::CortexError,
    artifacts::pool::{ByAddress, ByChainId, ByExecutionId, ShardPool},
    artifacts::retry::{RetryDecision, retry_with_backoff},
    state::StatusRegistry,
};
use kernel::{
    traits::{Broadcaster, IntentRelay, NonceManager, Signer, StateStore, Validator},
    types::{
        BroadcastError, ClientConfig, Eip1559Transaction, ExecutionId, NonceState, PipelineStatus,
        ValidatorOutcome,
    },
};
use std::sync::Arc;
use std::time::Instant;
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
/// |             | MissingProvider → immediate exit          |
/// | Validator   | Release nonce via `resolve(false)` → exit |
///
/// ## Nonce mismatch recovery
///
/// When broadcast returns `BroadcastError::NonceTooLow`, the pipeline:
/// 1. Immediately exits retry loop (no redundant attempts)
/// 2. Releases the incorrect nonce reservation
/// 3. Reverts signing status
/// 4. Syncs with on-chain nonce state
/// 5. Re-signs transaction with corrected nonce
/// 6. Retries broadcast once
/// 7. If second broadcast fails → pipeline fails
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
        // Start timing for this pipeline execution
        let start = Instant::now();
        tracing::info!(elapsed_ms = start.elapsed().as_millis(), "Pipeline started");

        // ============================================================
        // relay host

        let rh_result = retry_with_backoff(&ctx.retry_config, "relay_host", || {
            let rh = Arc::clone(&ctx.relayhost_handle);
            let txn = ctx.txn.clone();
            let cc = ctx.client_config.clone();

            async move {
                rh.register_transaction(execution_id, txn, cc)
                    .await
                    .map_err(RetryDecision::Retry)
            }
        })
        .await;

        if let Err(e) = rh_result {
            let err = CortexError::RelayHost(e);
            record_faliure(&ctx.status, execution_id, &err, &start);
            return;
        }

        ctx.status.set(execution_id, PipelineStatus::Accepted);
        tracing::debug!(
            elapsed_ms = start.elapsed().as_millis(),
            "relay_host: recorded"
        );

        // ============================================================
        // nonce reserve

        // geting the nonce handle from shard pool (squenced by from_address)
        let nonce_handle = ctx.nonce_pool.get(&ByAddress(&from_address));

        let nonce = match retry_with_backoff(&ctx.retry_config, "nonce_reserve", || {
            let nh = Arc::clone(&nonce_handle);
            async move {
                nh.reserve(chain_id, from_address, execution_id)
                    .await
                    .map_err(RetryDecision::Retry)
            }
        })
        .await
        {
            Ok(n) => n,
            Err(e) => {
                let err = CortexError::NonceReservation(e);
                record_faliure(&ctx.status, execution_id, &err, &start);
                return;
            }
        };

        ctx.status.set(execution_id, PipelineStatus::NonceReserved);
        tracing::info!(
            elapsed_ms = start.elapsed().as_millis(),
            nonce = %nonce,
            "nonce reserved"
        );

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
            async move {
                sh.sign(chain_id, from_address, execution_id, t)
                    .await
                    .map_err(RetryDecision::Retry)
            }
        })
        .await
        {
            Ok(s) => s,
            Err(e) => {
                // signing error -> hard fail
                release_nonce(&nonce_handle, execution_id, &ctx.retry_config, &start).await;

                let err = CortexError::Sign(e);
                record_faliure(&ctx.status, execution_id, &err, &start);
                return;
            }
        };

        ctx.status.set(execution_id, PipelineStatus::Signed);
        tracing::info!(
            elapsed_ms = start.elapsed().as_millis(),
            "transaction signed"
        );

        // ============================================================
        // broadcast (with nonce mismatch retry)

        // getting the broadcast handle from shard pool (sequenced by chain_id))
        let broadcast_handle = ctx.broadcast_pool.get(&ByChainId(&chain_id));
        let mut nonce_retry_attempted = false;

        let outcome = loop {
            match retry_with_backoff(&ctx.retry_config, "broadcast", || {
                let bh = Arc::clone(&broadcast_handle);
                let signed_clone = signed.clone();

                async move {
                    match bh
                        .broadcast(chain_id, from_address, execution_id, signed_clone)
                        .await
                    {
                        Ok(result) => Ok(result),
                        // Non-retryable errors that need special handling
                        Err(BroadcastError::NonceTooLow {
                            nonce_on_chain,
                            attempted_nonce,
                        }) => Err(RetryDecision::FailImmediately(
                            BroadcastError::NonceTooLow {
                                nonce_on_chain,
                                attempted_nonce,
                            },
                        )),
                        Err(BroadcastError::MissingProvider { chain_id }) => {
                            Err(RetryDecision::FailImmediately(
                                BroadcastError::MissingProvider { chain_id },
                            ))
                        }
                        // All other errors are retryable
                        Err(e) => Err(RetryDecision::Retry(e)),
                    }
                }
            })
            .await
            {
                Ok(broadcast_outcome) => break broadcast_outcome,

                // ============================================================
                // NonceTooLow -> Attempt one-time recovery
                Err(BroadcastError::NonceTooLow {
                    nonce_on_chain,
                    attempted_nonce,
                }) => {
                    if nonce_retry_attempted {
                        // Already retried once, give up to prevent infinite loop

                        tracing::error!(
                            elapsed_ms = start.elapsed().as_millis(),
                            nonce_on_chain = %nonce_on_chain,
                            attempted_nonce = %attempted_nonce,
                            "nonce matching failed after retry, aborting pipeline"
                        );

                        release_nonce(&nonce_handle, execution_id, &ctx.retry_config, &start).await;

                        let err = CortexError::Broadcast(BroadcastError::NonceTooLow {
                            nonce_on_chain,
                            attempted_nonce,
                        });
                        record_faliure(&ctx.status, execution_id, &err, &start);
                        return;
                    }

                    nonce_retry_attempted = true;

                    tracing::warn!(
                        elapsed_ms = start.elapsed().as_millis(),
                        attempted_nonce = %attempted_nonce,
                        nonce_on_chain = %nonce_on_chain,
                        "nonce mismatch detected - initiating sync"
                    );

                    // Update status to indicate nonce sync is happening

                    ctx.status.set(
                        execution_id,
                        PipelineStatus::NonceMismatchDetected {
                            nonce_on_chain,
                            attempted_nonce,
                        },
                    );

                    // Step 1: Release the incorrect nonce and revert sign status.

                    consume_nonce(&nonce_handle, execution_id, &ctx.retry_config, &start).await;
                    revert_sign(&sign_handle, execution_id, &ctx.retry_config, &start).await;
                    tracing::debug!(
                        elapsed_ms = start.elapsed().as_millis(),
                        "released incorrect nonce and reverted sign status"
                    );

                    // Step 2: Sync with on-chain state and reserve correct nonce

                    let corrected_nonce =
                        match retry_with_backoff(&ctx.retry_config, "nonce_sync", || {
                            let nh = Arc::clone(&nonce_handle);
                            async move {
                                nh.sync(chain_id, from_address, execution_id, nonce_on_chain)
                                    .await
                                    .map_err(RetryDecision::Retry)
                            }
                        })
                        .await
                        {
                            Ok(n) => n,
                            Err(e) => {
                                let err = CortexError::NonceSync(e);
                                record_faliure(&ctx.status, execution_id, &err, &start);
                                return;
                            }
                        };

                    tracing::info!(
                        elapsed_ms = start.elapsed().as_millis(),
                        corrected_nonce = %corrected_nonce,
                        "nonce synced and reserved after on-chain mismatch"
                    );
                    ctx.status.set(execution_id, PipelineStatus::NonceReserved);

                    // Step 3: Update transaction with corrected nonce

                    txn.nonce = corrected_nonce;

                    // Step 4: Re-sign transaction with corrected nonce

                    signed =
                        match retry_with_backoff(&ctx.retry_config, "sign_post_nonce_sync", || {
                            let sh = Arc::clone(&sign_handle);
                            let t = txn.clone();
                            async move {
                                sh.sign(chain_id, from_address, execution_id, t)
                                    .await
                                    .map_err(RetryDecision::Retry)
                            }
                        })
                        .await
                        {
                            Ok(s) => s,
                            Err(e) => {
                                release_nonce(
                                    &nonce_handle,
                                    execution_id,
                                    &ctx.retry_config,
                                    &start,
                                )
                                .await;

                                let err = CortexError::ReSign(e);
                                record_faliure(&ctx.status, execution_id, &err, &start);
                                return;
                            }
                        };

                    tracing::info!(
                        elapsed_ms = start.elapsed().as_millis(),
                        "transaction re-signed with corrected nonce"
                    );
                    ctx.status.set(execution_id, PipelineStatus::Signed);

                    // Step 5: Loop will retry broadcast with new signed transaction

                    tracing::debug!(
                        elapsed_ms = start.elapsed().as_millis(),
                        "retrying broadcast with corrected nonce"
                    );
                    continue;
                }

                // ============================================================
                // MissingProvider -> Immediate fatal failure
                Err(BroadcastError::MissingProvider { chain_id }) => {
                    release_nonce(&nonce_handle, execution_id, &ctx.retry_config, &start).await;

                    let err = CortexError::Broadcast(BroadcastError::MissingProvider { chain_id });
                    record_faliure(&ctx.status, execution_id, &err, &start);
                    return;
                }

                // ============================================================
                // Other broadcast errors -> Hard fail
                Err(e) => {
                    release_nonce(&nonce_handle, execution_id, &ctx.retry_config, &start).await;
                    let err = CortexError::Broadcast(e);
                    record_faliure(&ctx.status, execution_id, &err, &start);
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
            elapsed_ms = start.elapsed().as_millis(),
            tx_hash = %format!("{:#x}", tx_hash),
            "transaction broadcasted, waiting for confirmation on-chain:"
        );

        // ============================================================
        // validator

        let v = Arc::clone(&ctx.validator_handle);
        let validation =
            match { async move { v.validate(chain_id, execution_id, tx_hash).await } }.await {
                Ok(outcome) => outcome,
                Err(e) => {
                    release_nonce(&nonce_handle, execution_id, &ctx.retry_config, &start).await;
                    let err = CortexError::NotIncluded(e);
                    record_faliure(&ctx.status, execution_id, &err, &start);
                    return;
                }
            };

        match validation {
            ValidatorOutcome::Included {
                block_number,
                confirmations,
            } => {
                finalise_nonce(&nonce_handle, execution_id, &ctx.retry_config, &start).await;

                ctx.status.set(
                    execution_id,
                    PipelineStatus::ConfirmedOnChain {
                        tx_hash: format!("{tx_hash:#x}"),
                    },
                );
                tracing::info!(
                    elapsed_ms = start.elapsed().as_millis(),
                    block_number = %block_number,
                    confirmations = %confirmations,
                    "transaction confirmed on-chain"
                );
            }
            ValidatorOutcome::NotIncluded => {
                tracing::warn!(
                    elapsed_ms = start.elapsed().as_millis(),
                    tx_hash = %format!("{:#x}", tx_hash),
                    "transaction not included (reorg or eviction)"
                );
                release_nonce(&nonce_handle, execution_id, &ctx.retry_config, &start).await;

                ctx.status.set(
                    execution_id,
                    PipelineStatus::Failed {
                        stage: "validator".to_owned(),
                        reason: format!("tx {tx_hash:#x} not included on-chain"),
                    },
                );
            }
            ValidatorOutcome::Timeout => {
                tracing::warn!(
                    elapsed_ms = start.elapsed().as_millis(),
                    tx_hash = %format!("{:#x}", tx_hash),
                    "inclusion confimation failed (nonce gap or rpc failiure)"
                );
                consume_nonce(&nonce_handle, execution_id, &ctx.retry_config, &start).await;

                ctx.status.set(
                    execution_id,
                    PipelineStatus::ValidatorTimedOut {
                        message: "validator timed out, wait for confirmation".to_string(),
                    },
                );
            }
        }

        tracing::info!(
            elapsed_ms = start.elapsed().as_millis(),
            "pipeline completed"
        );
    }
    .instrument(span)
    .await;
}

// ============================================================
// helper

/// Release (invalidate) a reserved nonce so it can be re-used.
/// Called on any hard-fail *after* a nonce has been reserved.
async fn release_nonce(
    handle: &Arc<dyn NonceManager>,
    execution_id: ExecutionId,
    retry_config: &RetryConfig,
    start: &Instant,
) {
    let resolve_result = retry_with_backoff(&retry_config, "nonce_release", || {
        let nh = Arc::clone(handle);
        async move {
            nh.resolve(execution_id, NonceState::Released)
                .await
                .map_err(RetryDecision::Retry)
        }
    })
    .await;

    if let Err(e) = resolve_result {
        // not fatal -> lease will expire in 5 minutes
        tracing::error!(
            elapsed_ms = start.elapsed().as_millis(),
            error = %e,
            "nonce release failed after retries, lease will expire in 5 min"
        );
    } else {
        tracing::debug!(elapsed_ms = start.elapsed().as_millis(), "nonce released");
    }
}

/// Revert sign status from 'signed' to 'failed'
/// for error handeling pathways that require re-signing.
/// **example**: `NoneTooLow`.
async fn revert_sign(
    handle: &Arc<dyn Signer>,
    execution_id: ExecutionId,
    retry_config: &RetryConfig,
    start: &Instant,
) {
    let revert_result = retry_with_backoff(&retry_config, "sign_revert", || {
        let sh = Arc::clone(handle);
        async move { sh.revert(execution_id).await.map_err(RetryDecision::Retry) }
    })
    .await;

    if let Err(e) = revert_result {
        // not fatal -> lease will expire in 5 minutes
        tracing::error!(
            elapsed_ms = start.elapsed().as_millis(),
            error = %e,
            "sign state revertion failed"
        );
    } else {
        tracing::debug!(
            elapsed_ms = start.elapsed().as_millis(),
            "sign state set to failed"
        );
    }
}

/// Finalize a reserved nonce after on-chain inclusion.
async fn finalise_nonce(
    handle: &Arc<dyn NonceManager>,
    execution_id: ExecutionId,
    retry_config: &RetryConfig,
    start: &Instant,
) {
    let resolve_result = retry_with_backoff(&retry_config, "nonce_finalise", || {
        let nh = Arc::clone(handle);
        async move {
            nh.resolve(execution_id, NonceState::Finalized)
                .await
                .map_err(RetryDecision::Retry)
        }
    })
    .await;

    if let Err(e) = resolve_result {
        // not fatal -> dangling nonce lease expires in 5 min
        tracing::error!(
            elapsed_ms = start.elapsed().as_millis(),
            error = %e,
            "nonce finalising failed, state remains 'reserved', lease will expire in 5 minutes"
        );
    } else {
        tracing::debug!(elapsed_ms = start.elapsed().as_millis(), "nonce finalised");
    }
}

/// Consume a 'reserved' nonce, in case a nonce gap is created on-chain
/// and the nonce overflows, the validator will not be able
/// to confirm inclusion of nonce on chain, until gap is covered.
async fn consume_nonce(
    handle: &Arc<dyn NonceManager>,
    execution_id: ExecutionId,
    retry_config: &RetryConfig,
    start: &Instant,
) {
    let resolve_result = retry_with_backoff(&retry_config, "nonce_release", || {
        let nh = Arc::clone(handle);
        async move {
            nh.resolve(execution_id, NonceState::Consumed)
                .await
                .map_err(RetryDecision::Retry)
        }
    })
    .await;

    if let Err(e) = resolve_result {
        // not fatal -> lease will expire in 5 minutes
        tracing::error!(
            elapsed_ms = start.elapsed().as_millis(),
            error = %e,
            "'consume' state update failed after retries, 'reserve' lease will expire in 5 min"
        );
    } else {
        tracing::debug!(elapsed_ms = start.elapsed().as_millis(), "nonce released");
    }
}

/// helper to record fatal errors.
fn record_faliure(
    registry: &StatusRegistry,
    execution_id: ExecutionId,
    err: &CortexError,
    start: &Instant,
) {
    tracing::error!(
        elapsed_ms = start.elapsed().as_millis(),
        stage = %err.stage(),
        error = %err,
        "pipeline hard fail"
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
