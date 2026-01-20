use crate::types::*;
use alloy_primitives::Address;
use async_trait::async_trait;

// ============================================================

#[async_trait]
pub trait Pipeline: Send + Sync + 'static {
    async fn submit(&self, intent: Intent) -> Result<IntentResult, ExecutionError>;
}

// ============================================================

#[async_trait]
pub trait StateStore: Send + Sync {
    async fn register_intent(&self, intent: Intent) -> Result<Execution, ExecutionError>;

    async fn record_nonce(&self, id: ExecutionId, nonce: TxNonce) -> Result<(), ExecutionError>;

    async fn record_raw_tx(
        &self,
        id: ExecutionId,
        tx: &RawTransaction,
    ) -> Result<(), ExecutionError>;

    async fn record_signed_tx(
        &self,
        id: ExecutionId,
        tx: &SignedTransaction,
    ) -> Result<(), ExecutionError>;

    async fn mark_broadcasted(
        &self,
        id: ExecutionId,
        tx_hash: TxHash,
    ) -> Result<(), ExecutionError>;

    async fn transition(
        &self,
        id: ExecutionId,
        state: ExecutionState,
    ) -> Result<(), ExecutionError>;

    async fn mark_failed(
        &self,
        id: ExecutionId,
        error: ExecutionError,
    ) -> Result<(), ExecutionError>;
}

// ============================================================

#[async_trait]
pub trait NonceManager: Send + Sync {
    /// Reserve the next available nonce (provisional).
    async fn reserve(&self, chain_id: ChainId, from: Address) -> Result<TxNonce, ExecutionError>;

    /// Resolve a nonce after finality (success = confirmed, false = dropped).
    async fn resolve(
        &self,
        chain_id: ChainId,
        from: Address,
        nonce: TxNonce,
        success: bool,
    ) -> Result<(), ExecutionError>;

    /// Explicitly drop a nonce (broadcast failed or abandoned).
    async fn drop(
        &self,
        chain_id: ChainId,
        from: Address,
        nonce: TxNonce,
    ) -> Result<(), ExecutionError>;
}

// ============================================================

#[async_trait]
pub trait Canonicalizer: Send + Sync {
    async fn canonicalize(
        &self,
        intent: &SendTransactionIntent,
        chain_id: ChainId,
        nonce: TxNonce,
    ) -> Result<RawTransaction, ExecutionError>;
}

// ============================================================

#[async_trait]
pub trait Signer: Send + Sync {
    async fn sign(
        &self,
        chain_id: ChainId,
        from: Address,
        tx: &RawTransaction,
    ) -> Result<SignedTransaction, ExecutionError>;
}

// ============================================================

#[async_trait]
pub trait Broadcaster: Send + Sync {
    async fn broadcast(
        &self,
        chain_id: ChainId,
        tx: &SignedTransaction,
    ) -> Result<BroadcastOutcome, ExecutionError>;
}

// ============================================================

#[async_trait]
pub trait Validator: Send + Sync {
    async fn watch(&self, chain_id: ChainId, tx_hash: TxHash) -> Result<bool, ExecutionError>;
}

// ============================================================
