use crate::types::*;
use alloy_primitives::{Address, U256};
use async_trait::async_trait;
use rlp::RlpStream;

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
        tx: &Eip1559Transaction,
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
    async fn reserve(
        &self,
        chain_id: ChainId,
        from: Address,
        id: ExecutionId,
    ) -> Result<TxNonce, ExecutionError>;

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
    ) -> Result<Eip1559Transaction, ExecutionError>;
}

// ============================================================

#[async_trait]
pub trait Signer: Send + Sync {
    async fn sign(
        &self,
        from: Address,
        chain_id: ChainId,
        execution_id: ExecutionId,
        tx: Eip1559Transaction,
    ) -> Result<SignedTransaction, ExecutionError>;
}

// ============================================================

pub trait PolicyEngine: Send + Sync {
    fn resolve_key(&self, from: &Address) -> Result<(String, [u8; 32]), ExecutionError>;
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
// Ethereum-specific RLP encoding.
//
// This trait defines how a value is appended to an RLP stream
// following Ethereum consensus rules (minimal big-endian, zero rules, etc).

pub trait EthRlpEncode {
    fn eth_rlp_append(&self, s: &mut rlp::RlpStream);
}

// ============================================================
// Important function for EthRlpEncode trait implimentation on TxNonce, Chain_ID, U256 and Address type.

pub fn eth_rlp_append_u256(value: &U256, s: &mut RlpStream) {
    if value.is_zero() {
        s.append_empty_data();
    } else {
        let buf: [u8; 32] = value.to_be_bytes();

        let first_non_zero = buf.iter().position(|b| *b != 0).unwrap();
        s.encoder().encode_value(&buf[first_non_zero..]);
    }
}

// ============================================================

impl EthRlpEncode for U256 {
    fn eth_rlp_append(&self, s: &mut rlp::RlpStream) {
        eth_rlp_append_u256(self, s);
    }
}

impl EthRlpEncode for Address {
    fn eth_rlp_append(&self, s: &mut rlp::RlpStream) {
        s.encoder().encode_value(self.as_slice());
    }
}

// ============================================================
