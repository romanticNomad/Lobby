//! High-level transaction orchestration with sticky session affinity
//!
//! Provides `TransactionService` for EVM transaction submission and receipt polling
//! with endpoint affinity guarantees for consistent mempool views.

use crate::rpc::client::{EndpointStats, RpcClient, RpcError};
use alloy::{
    primitives::{Bytes, TxHash},
    rpc::types::TransactionReceipt,
};
use primitives::types::ChainId;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinSet,
    time::sleep,
};
use tracing::{debug, error, info, trace, warn};

// ============================================================================
// Constants

/// Default polling interval for transaction receipts (milliseconds)
const DEFAULT_POLL_INTERVAL_MS: u64 = 250;

/// Maximum polling duration before giving up (seconds)
const DEFAULT_MAX_POLL_DURATION_SECS: u64 = 300;

/// Backoff multiplier for polling intervals
const DEFAULT_BACKOFF_MULTIPLIER: f64 = 1.5;

/// Maximum polling interval cap (milliseconds)
const MAX_POLL_INTERVAL_MS: u64 = 8000;

/// Batch submission concurrency limit per batch
const BATCH_CONCURRENCY_LIMIT: usize = 100;

/// Sticky session retry threshold - if endpoint fails this many times, abandon stickiness
const STICKY_RETRY_THRESHOLD: u32 = 3;

// ============================================================================
// Configuration Types

/// Configuration for receipt polling behavior
#[derive(Debug, Clone)]
pub struct PollConfig {
    /// Initial polling interval
    pub initial_interval: Duration,
    /// Maximum time to poll before giving up
    pub max_duration: Duration,
    /// Backoff multiplier between polls
    pub backoff_multiplier: f64,
    /// Maximum interval between polls
    pub max_interval: Duration,
    /// Whether to use sticky endpoint affinity
    pub use_sticky_affinity: bool,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            initial_interval: Duration::from_millis(DEFAULT_POLL_INTERVAL_MS),
            max_duration: Duration::from_secs(DEFAULT_MAX_POLL_DURATION_SECS),
            backoff_multiplier: DEFAULT_BACKOFF_MULTIPLIER,
            max_interval: Duration::from_millis(MAX_POLL_INTERVAL_MS),
            use_sticky_affinity: true,
        }
    }
}

impl PollConfig {
    /// Creates a new poll config with custom parameters
    pub fn new(initial_interval_ms: u64, max_duration_secs: u64, backoff_multiplier: f64) -> Self {
        Self {
            initial_interval: Duration::from_millis(initial_interval_ms),
            max_duration: Duration::from_secs(max_duration_secs),
            backoff_multiplier: backoff_multiplier.max(1.0),
            max_interval: Duration::from_millis(MAX_POLL_INTERVAL_MS),
            use_sticky_affinity: true,
        }
    }

    /// Disables sticky affinity for polling
    pub fn without_sticky_affinity(mut self) -> Self {
        self.use_sticky_affinity = false;
        self
    }
}

// ============================================================================
// Transaction Service

/// High-level service for EVM transaction orchestration
///
/// Provides:
/// - Raw transaction submission with endpoint affinity tracking
/// - Receipt polling with sticky session guarantees
/// - Batch submission with parallel execution
/// - Background polling for fire-and-forget submissions
#[derive(Clone)]
pub struct TransactionService {
    /// Underlying RPC client for provider access
    client: Arc<RpcClient>,
    /// Default timeout for operations
    default_timeout: Duration,
    /// Default polling configuration
    default_poll_config: PollConfig,
}

/// Context returned after transaction submission for receipt tracking
#[derive(Debug, Clone)]
pub struct TransactionContext {
    /// Transaction hash
    pub tx_hash: TxHash,
    /// Chain ID where submitted
    pub chain_id: ChainId,
    /// Endpoint index for sticky affinity
    pub endpoint_index: usize,
    /// Timestamp of submission
    pub submitted_at: Instant,
}

/// Result of a single transaction submission
#[derive(Debug, Clone)]
pub struct SubmissionResult {
    /// Transaction context for tracking
    pub context: TransactionContext,
    /// Whether submission was successful
    pub success: bool,
    /// Error message if failed
    pub error: Option<String>,
}

/// Result of batch transaction submission
#[derive(Debug, Clone)]
pub struct BatchSubmissionResult {
    /// Per-transaction results with endpoint attribution
    pub results: Vec<SubmissionResult>,
    /// Total time for batch processing
    pub total_duration: Duration,
    /// Number of successful submissions
    pub success_count: usize,
    /// Number of failed submissions
    pub failure_count: usize,
}

/// Receipt result with metadata
#[derive(Debug, Clone)]
pub struct ReceiptResult {
    /// Transaction hash that was polled
    pub tx_hash: TxHash,
    /// The receipt if found
    pub receipt: Option<TransactionReceipt>,
    /// Number of polling attempts made
    pub attempts: u32,
    /// Total time spent polling
    pub duration: Duration,
    /// Whether sticky affinity was maintained
    pub sticky_maintained: bool,
}

/// Background polling handle for fire-and-forget submissions
#[derive(Debug)]
pub struct BackgroundPollHandle {
    /// Receiver for poll completion
    pub result_rx: oneshot::Receiver<ReceiptResult>,
    /// Can be used to cancel polling
    pub cancel_tx: mpsc::Sender<()>,
}

// ============================================================================
// Implementation

impl TransactionService {
    /// Creates a new transaction service with the given RPC client
    pub fn new(client: Arc<RpcClient>) -> Self {
        Self {
            client,
            default_timeout: Duration::from_secs(30),
            default_poll_config: PollConfig::default(),
        }
    }

    /// Sets the default timeout for operations
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Sets the default polling configuration
    pub fn with_poll_config(mut self, config: PollConfig) -> Self {
        self.default_poll_config = config;
        self
    }

    // ========================================================================
    // Core Transaction Operations

    /// Submits a raw signed transaction to the network
    ///
    /// Returns the transaction hash and endpoint index for sticky affinity.
    /// The endpoint index should be stored and used for receipt polling.
    ///
    /// # Example
    /// ```ignore
    /// let (tx_hash, endpoint_index) = service.send_raw_transaction(chain_id, signed_tx).await?;
    /// // Store endpoint_index for later receipt polling
    /// ```
    pub async fn send_raw_transaction(
        &self,
        chain_id: ChainId,
        signed_tx: Bytes,
    ) -> Result<(TxHash, usize), RpcError> {
        let start = Instant::now();

        // Acquire unary context with weighted selection (no sticky index for initial submission)
        let (ctx, permit) = self
            .client
            .acquire_unary_context(&chain_id, None, self.default_timeout)
            .await?;

        let endpoint_id = ctx.endpoint_id().to_string();
        let endpoint_index = ctx.index();

        trace!(
            tx_hash_prefix = %format!("0x{}", hex::encode(&signed_tx[..4.min(signed_tx.len())])),
            chain_id = %chain_id,
            endpoint_id = %endpoint_id,
            endpoint_index = endpoint_index,
            "Submitting raw transaction"
        );

        // Submit transaction using Alloy provider
        let result = ctx.provider().send_raw_transaction(&signed_tx).await;

        match result {
            Ok(tx_result) => {
                let tx_hash = tx_result.tx_hash();
                let duration = start.elapsed();
                ctx.record_success(duration);

                info!(
                    tx_hash = %tx_hash,
                    chain_id = %chain_id,
                    endpoint_id = %endpoint_id,
                    endpoint_index = endpoint_index,
                    duration_ms = duration.as_millis() as u64,
                    "Transaction submitted successfully"
                );

                drop(permit);
                Ok((tx_hash.to_owned(), endpoint_index))
            }
            Err(e) => {
                ctx.record_failure();
                error!(
                    error = %e,
                    chain_id = %chain_id,
                    endpoint_id = %endpoint_id,
                    endpoint_index = endpoint_index,
                    "Transaction submission failed"
                );
                drop(permit);
                Err(RpcError::TransportError(e.to_string()))
            }
        }
    }

    /// Gets a transaction receipt with optional sticky endpoint affinity
    ///
    /// If `sticky_index` is provided, attempts to use that specific endpoint.
    /// Falls back to weighted selection if the endpoint is unhealthy.
    ///
    /// # Example
    /// ```ignore
    /// // Using sticky affinity from submission
    /// let receipt = service.get_transaction_receipt(chain_id, tx_hash, Some(endpoint_index)).await?;
    ///
    /// // Without sticky affinity (any endpoint)
    /// let receipt = service.get_transaction_receipt(chain_id, tx_hash, None).await?;
    /// ```
    pub async fn get_transaction_receipt(
        &self,
        chain_id: ChainId,
        tx_hash: TxHash,
        sticky_index: Option<usize>,
    ) -> Result<Option<TransactionReceipt>, RpcError> {
        let start = Instant::now();

        // Acquire context with sticky affinity if requested
        let (ctx, permit) = self
            .client
            .acquire_unary_context(&chain_id, sticky_index, self.default_timeout)
            .await?;

        let endpoint_id = ctx.endpoint_id().to_string();
        let actual_index = ctx.index();
        let _sticky_maintained = sticky_index.map(|si| si == actual_index).unwrap_or(true);

        trace!(
            tx_hash = %tx_hash,
            chain_id = %chain_id,
            endpoint_id = %endpoint_id,
            requested_sticky = ?sticky_index,
            actual_index = actual_index,
            "Fetching transaction receipt"
        );

        let result = ctx.provider().get_transaction_receipt(tx_hash).await;

        match result {
            Ok(receipt) => {
                let duration = start.elapsed();
                ctx.record_success(duration);

                if receipt.is_some() {
                    debug!(
                        tx_hash = %tx_hash,
                        chain_id = %chain_id,
                        endpoint_id = %endpoint_id,
                        block_number = ?receipt.as_ref().and_then(|r| r.block_number),
                        "Transaction receipt found"
                    );
                } else {
                    trace!(
                        tx_hash = %tx_hash,
                        chain_id = %chain_id,
                        endpoint_id = %endpoint_id,
                        "Transaction receipt not yet available"
                    );
                }

                drop(permit);
                Ok(receipt)
            }
            Err(e) => {
                ctx.record_failure();
                warn!(
                    tx_hash = %tx_hash,
                    chain_id = %chain_id,
                    endpoint_id = %endpoint_id,
                    error = %e,
                    "Failed to fetch transaction receipt"
                );
                drop(permit);
                Err(RpcError::TransportError(e.to_string()))
            }
        }
    }

    /// Polls for transaction receipt with exponential backoff
    ///
    /// Maintains sticky endpoint affinity throughout polling to ensure
    /// consistent mempool view. Uses backoff strategy to reduce RPC load.
    ///
    /// Returns `None` if receipt not found within `max_duration`.
    pub async fn poll_for_receipt(
        &self,
        chain_id: ChainId,
        tx_hash: TxHash,
        sticky_index: usize,
        config: PollConfig,
    ) -> ReceiptResult {
        let start = Instant::now();
        let mut attempts: u32 = 0;
        let mut current_interval = config.initial_interval;
        let mut current_sticky_index = Some(sticky_index);
        let mut sticky_abandoned = false;
        let mut sticky_retry_count = 0;

        trace!(
            tx_hash = %tx_hash,
            chain_id = %chain_id,
            sticky_index = sticky_index,
            max_duration_secs = config.max_duration.as_secs(),
            "Starting receipt polling"
        );

        while start.elapsed() < config.max_duration {
            attempts += 1;

            match self
                .get_transaction_receipt(chain_id, tx_hash, current_sticky_index)
                .await
            {
                Ok(Some(receipt)) => {
                    let duration = start.elapsed();
                    info!(
                        tx_hash = %tx_hash,
                        chain_id = %chain_id,
                        attempts = attempts,
                        duration_ms = duration.as_millis() as u64,
                        block_number = ?receipt.block_number,
                        "Transaction confirmed"
                    );

                    return ReceiptResult {
                        tx_hash,
                        receipt: Some(receipt),
                        attempts,
                        duration,
                        sticky_maintained: !sticky_abandoned,
                    };
                }
                Ok(None) => {
                    // Transaction pending, continue polling
                    trace!(
                        tx_hash = %tx_hash,
                        chain_id = %chain_id,
                        attempt = attempts,
                        next_poll_ms = current_interval.as_millis() as u64,
                        "Receipt not yet available, continuing poll"
                    );
                }
                Err(e) => {
                    // Error fetching receipt - may need to abandon sticky affinity
                    warn!(
                        tx_hash = %tx_hash,
                        chain_id = %chain_id,
                        error = %e,
                        sticky_retry_count = sticky_retry_count,
                        "Error polling for receipt"
                    );

                    if !sticky_abandoned {
                        sticky_retry_count += 1;
                        if sticky_retry_count >= STICKY_RETRY_THRESHOLD {
                            warn!(
                                tx_hash = %tx_hash,
                                chain_id = %chain_id,
                                "Abandoning sticky affinity due to repeated failures"
                            );
                            sticky_abandoned = true;
                            current_sticky_index = None;
                        }
                    }
                }
            }

            // Wait before next poll with backoff
            sleep(current_interval).await;

            // Calculate next interval with backoff
            let next_interval_ms = (current_interval.as_millis() as f64 * config.backoff_multiplier)
                .min(config.max_interval.as_millis() as f64)
                as u64;
            current_interval = Duration::from_millis(next_interval_ms);
        }

        // Timeout reached
        let duration = start.elapsed();
        warn!(
            tx_hash = %tx_hash,
            chain_id = %chain_id,
            attempts = attempts,
            duration_secs = duration.as_secs(),
            "Receipt polling timed out"
        );

        ReceiptResult {
            tx_hash,
            receipt: None,
            attempts,
            duration,
            sticky_maintained: !sticky_abandoned,
        }
    }

    /// Convenience method to submit and poll for receipt in one call
    ///
    /// Submits the transaction, then polls for receipt using sticky affinity.
    /// Returns the receipt once confirmed or timeout reached.
    pub async fn submit_and_wait_for_receipt(
        &self,
        chain_id: ChainId,
        signed_tx: Bytes,
        poll_config: Option<PollConfig>,
    ) -> Result<ReceiptResult, RpcError> {
        // Submit transaction
        let (tx_hash, endpoint_index) = self.send_raw_transaction(chain_id, signed_tx).await?;

        // Poll for receipt with sticky affinity
        let config = poll_config.unwrap_or(self.default_poll_config.clone());
        let result = self
            .poll_for_receipt(chain_id, tx_hash, endpoint_index, config)
            .await;

        Ok(result)
    }

    // ========================================================================
    // Batch Operations

    /// Submits multiple raw transactions in parallel
    ///
    /// Distributes load across endpoints using weighted selection.
    /// Returns per-transaction results with endpoint attribution.
    ///
    /// # Performance
    /// - Concurrent submissions limited by `BATCH_CONCURRENCY_LIMIT`
    /// - Each submission independently selects optimal endpoint
    /// - Total time: roughly O(n/concurrency) for n transactions
    pub async fn send_raw_transactions_batch(
        &self,
        chain_id: ChainId,
        signed_txs: Vec<Bytes>,
    ) -> BatchSubmissionResult {
        let batch_start = Instant::now();
        let tx_count = signed_txs.len();

        info!(
            chain_id = %chain_id,
            tx_count = tx_count,
            "Starting batch transaction submission"
        );

        // Create semaphore for concurrency limiting
        let semaphore = Arc::new(tokio::sync::Semaphore::new(BATCH_CONCURRENCY_LIMIT));
        let mut join_set = JoinSet::new();

        for (_idx, signed_tx) in signed_txs.into_iter().enumerate() {
            let service = self.clone();
            let sem = Arc::clone(&semaphore);
            let chain = chain_id;

            join_set.spawn(async move {
                let _permit = sem.acquire().await.expect("Semaphore should not close");
                let tx_start = Instant::now();

                match service.send_raw_transaction(chain, signed_tx).await {
                    Ok((tx_hash, endpoint_index)) => SubmissionResult {
                        context: TransactionContext {
                            tx_hash,
                            chain_id: chain,
                            endpoint_index,
                            submitted_at: tx_start,
                        },
                        success: true,
                        error: None,
                    },
                    Err(e) => {
                        SubmissionResult {
                            context: TransactionContext {
                                tx_hash: TxHash::ZERO, // Placeholder for failed tx
                                chain_id: chain,
                                endpoint_index: 0,
                                submitted_at: tx_start,
                            },
                            success: false,
                            error: Some(e.to_string()),
                        }
                    }
                }
            });
        }

        // Collect all results
        let mut results = Vec::with_capacity(tx_count);
        let mut success_count = 0;
        let mut failure_count = 0;

        while let Some(Ok(result)) = join_set.join_next().await {
            if result.success {
                success_count += 1;
            } else {
                failure_count += 1;
            }
            results.push(result);
        }

        let total_duration = batch_start.elapsed();

        info!(
            chain_id = %chain_id,
            tx_count = tx_count,
            success_count = success_count,
            failure_count = failure_count,
            duration_ms = total_duration.as_millis() as u64,
            "Batch submission complete"
        );

        BatchSubmissionResult {
            results,
            total_duration,
            success_count,
            failure_count,
        }
    }

    /// Polls for multiple receipts in parallel with sticky affinity
    ///
    /// Each receipt is polled on its original submission endpoint.
    /// Returns results as they complete (order not guaranteed).
    pub async fn poll_for_receipts_batch(
        &self,
        contexts: Vec<TransactionContext>,
        poll_config: Option<PollConfig>,
    ) -> Vec<ReceiptResult> {
        let poll_start = Instant::now();
        let ctx_count = contexts.len();

        info!(tx_count = ctx_count, "Starting batch receipt polling");

        let mut join_set = JoinSet::new();
        let config = poll_config.unwrap_or(self.default_poll_config.clone());

        for ctx in contexts {
            let service = self.clone();
            let cfg = config.clone();

            join_set.spawn(async move {
                service
                    .poll_for_receipt(ctx.chain_id, ctx.tx_hash, ctx.endpoint_index, cfg)
                    .await
            });
        }

        let mut results = Vec::with_capacity(ctx_count);
        while let Some(Ok(result)) = join_set.join_next().await {
            results.push(result);
        }

        info!(
            tx_count = ctx_count,
            confirmed_count = results.iter().filter(|r| r.receipt.is_some()).count(),
            duration_ms = poll_start.elapsed().as_millis() as u64,
            "Batch polling complete"
        );

        results
    }

    // ========================================================================
    // Background Operations

    /// Submits a transaction and returns immediately, polling in background
    ///
    /// Returns a handle to receive the receipt result when available.
    /// Useful for fire-and-forget patterns where you don't want to block.
    pub async fn submit_and_poll_background(
        &self,
        chain_id: ChainId,
        signed_tx: Bytes,
        poll_config: Option<PollConfig>,
    ) -> Result<BackgroundPollHandle, RpcError> {
        // Submit transaction first
        let (tx_hash, endpoint_index) = self.send_raw_transaction(chain_id, signed_tx).await?;

        // Create channels for result and cancellation
        let (result_tx, result_rx) = oneshot::channel();
        let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(1);

        let service = self.clone();
        let config = poll_config.unwrap_or(self.default_poll_config.clone());

        // Spawn background polling task
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.initial_interval);
            let start = Instant::now();
            let mut attempts: u32 = 0;
            let mut current_sticky_index = Some(endpoint_index);
            let mut sticky_abandoned = false;
            let mut sticky_retry_count = 0;

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if start.elapsed() >= config.max_duration {
                            let _ = result_tx.send(ReceiptResult {
                                tx_hash,
                                receipt: None,
                                attempts,
                                duration: start.elapsed(),
                                sticky_maintained: !sticky_abandoned,
                            });
                            break;
                        }

                        attempts += 1;

                        match service.get_transaction_receipt(chain_id, tx_hash, current_sticky_index).await {
                            Ok(Some(receipt)) => {
                                let _ = result_tx.send(ReceiptResult {
                                    tx_hash,
                                    receipt: Some(receipt),
                                    attempts,
                                    duration: start.elapsed(),
                                    sticky_maintained: !sticky_abandoned,
                                });
                                break;
                            }
                            Ok(None) => {
                                // Continue polling
                            }
                            Err(_) => {
                                if !sticky_abandoned {
                                    sticky_retry_count += 1;
                                    if sticky_retry_count >= STICKY_RETRY_THRESHOLD {
                                        sticky_abandoned = true;
                                        current_sticky_index = None;
                                    }
                                }
                            }
                        }
                    }
                    _ = cancel_rx.recv() => {
                        trace!(tx_hash = %tx_hash, "Background polling cancelled");
                        break;
                    }
                }
            }
        });

        Ok(BackgroundPollHandle {
            result_rx,
            cancel_tx,
        })
    }

    // ========================================================================
    // Utility Methods

    /// Gets the current unary endpoint statistics for a chain
    pub async fn get_unary_endpoint_stats(&self, chain_id: &ChainId) -> Option<Vec<EndpointStats>> {
        self.client.get_unary_endpoint_stats(chain_id).await
    }

    /// Gets the current subscription endpoint statistics for a chain
    pub async fn get_subscription_endpoint_stats(
        &self,
        chain_id: &ChainId,
    ) -> Option<Vec<EndpointStats>> {
        self.client.get_subscription_endpoint_stats(chain_id).await
    }

    /// Gets the number of available permits in the semaphore
    pub fn available_concurrency(&self) -> usize {
        self.client.available_permits()
    }

    /// Gets the registered chain count
    pub fn registered_chains(&self) -> usize {
        self.client.total_registered_chains()
    }
}

// ============================================================================
// Additional Helper Types and Functions

/// Builder pattern for complex transaction operations
pub struct TransactionBuilder {
    chain_id: ChainId,
    service: Arc<TransactionService>,
}

impl TransactionBuilder {
    /// Creates a new builder for the specified chain
    pub fn new(chain_id: ChainId, service: Arc<TransactionService>) -> Self {
        Self { chain_id, service }
    }

    /// Submits a raw transaction and returns the context
    pub async fn submit_raw(self, signed_tx: Bytes) -> Result<TransactionContext, RpcError> {
        let (tx_hash, endpoint_index) = self
            .service
            .send_raw_transaction(self.chain_id, signed_tx)
            .await?;
        Ok(TransactionContext {
            tx_hash,
            chain_id: self.chain_id,
            endpoint_index,
            submitted_at: Instant::now(),
        })
    }
}

// ============================================================================
