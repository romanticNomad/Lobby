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

    async fn mark_final(&self, id: ExecutionId, success: bool) -> Result<(), ExecutionError>;
}

// ============================================================

#[async_trait]
pub trait NonceManager: Send + Sync {
    /// Reserve the next available nonce (provisional).
    async fn reserve(
        &self,
        chain_id: ChainId,
        from: Address,
        id: ExecutionId,
    ) -> Result<TxNonce, ExecutionError>;

    /// Resolve a nonce after validation (success = confirmed, false = dropped).
    async fn resolve(&self, id: ExecutionId, outcome: bool) -> Result<(), ExecutionError>;
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
        from: Address,
        chain_id: ChainId,
        id: ExecutionId,
        tx: RawTransaction,
    ) -> Result<SignedTransaction, ExecutionError>;
}

// ============================================================

#[async_trait]
pub trait Broadcaster: Send + Sync {
    async fn broadcast(
        &self,
        chain_id: ChainId,
        id: ExecutionId,
        tx: SignedTransaction,
    ) -> Result<BroadcastOutcome, ExecutionError>;
}

// ============================================================

#[async_trait]
pub trait Validator: Send + Sync {
    async fn watch(&self, chain_id: ChainId, tx_hash: TxHash) -> Result<bool, ExecutionError>;
}

// ============================================================
