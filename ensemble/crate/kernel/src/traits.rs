use crate::types::*;
use alloy::primitives::{Address, U256};
use async_trait::async_trait;
use rlp::RlpStream;

// ============================================================

#[async_trait]
pub trait NonceManager: Send + Sync {
    async fn reserve(
        &self,
        chain_id: ChainId,
        from_address: Address,
        execution_id: ExecutionId,
    ) -> Result<TxNonce, LocalError>;

    async fn resolve(&self, execution_id: ExecutionId, state: NonceState) -> Result<(), LocalError>;

    /// special api for syncing nonce of given (chain_id, from_address)
    /// in case the lobby DB and on-chain state are detected to be out of sync.
    async fn sync(
        &self,
        chain_id: ChainId,
        from_address: Address,
        execution_id: ExecutionId,
        nonce_on_chain: TxNonce,
    ) -> Result<TxNonce, LocalError>;
}

// ============================================================

#[async_trait]
pub trait Signer: Send + Sync {
    async fn sign(
        &self,
        chain_id: ChainId,
        from_address: Address,
        execution_id: ExecutionId,
        txn: Eip1559Transaction,
    ) -> Result<SignedTransaction, LocalError>;

    /// changing status to 'failed' for resigning transaction
    /// after resolving transaction issues like nonce_sync
    async fn revert(&self, execution_id: ExecutionId) -> Result<(), LocalError>;
}

// ============================================================

pub trait PolicyEngine: Send + Sync {
    fn resolve_key(&self, from_address: &Address) -> Result<[u8; 32], LocalError>;
}

// ============================================================

#[async_trait]
pub trait Broadcaster: Send + Sync {
    async fn broadcast(
        &self,
        chain_id: ChainId,
        from_address: Address,
        execution_id: ExecutionId,
        txn: SignedTransaction,
    ) -> Result<BroadcastOutcome, BroadcastError>;
}

// ============================================================

#[async_trait]
pub trait Validator: Send + Sync {
    async fn validate(
        &self,
        chain_id: ChainId,
        execution_id: ExecutionId,
        tx_hash: TxHash,
    ) -> Result<ValidatorOutcome, ValidatorError>;
}

// ============================================================
// Checks whether a broadcast transaction has been included in a block.

#[async_trait]
pub trait IntentRelay: Send + Sync {
    async fn register_transaction(
        &self,
        execution_id: ExecutionId,
        txn: Eip1559Transaction,
        client_config: ClientConfig,
    ) -> Result<(), RelayHostError>;
}

// ============================================================
// Ethereum-specific RLP encoding.
///
/// This trait defines how a value is appended to an RLP stream
/// following Ethereum consensus rules (minimal big-endian, zero rules, etc).

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
        let trimmed = &buf[first_non_zero..];

        // Encode into a temporary stream so the prefix bytes are written,
        // then append_raw the result (1 item) into the parent stream.
        // This ensures note_appended(1) is called on the parent list.
        let mut tmp = RlpStream::new();
        tmp.encoder().encode_value(trimmed);
        s.append_raw(&tmp.as_raw().to_vec(), 1);
    }
}

// ============================================================
// implimenting EthRlpEncode to encode U256 and Address types into RlpStream

impl EthRlpEncode for U256 {
    fn eth_rlp_append(&self, s: &mut rlp::RlpStream) {
        eth_rlp_append_u256(self, s);
    }
}

impl EthRlpEncode for Address {
    fn eth_rlp_append(&self, s: &mut rlp::RlpStream) {
        // Encode into a temporary stream, then append_raw 1 item into the
        // parent so note_appended(1) is called and the list counter advances.
        let mut tmp = RlpStream::new();
        tmp.encoder().encode_value(self.as_slice());
        s.append_raw(&tmp.as_raw().to_vec(), 1);
    }
}

// ============================================================
