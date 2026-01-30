use crate::rlp::{encode_eip1559_signed, encode_eip1559_unsigned};
use alloy_primitives::bytes::Bytes;
use k256::{
    ecdsa::{RecoveryId, Signature, SigningKey},
    elliptic_curve::scalar::IsHigh,
};
use kernel::types::{Eip1559Transaction, ExecutionError, SignedTransaction};
use sha3::{Digest, Keccak256};
use zeroize::Zeroize;

// ============================================================

pub fn sign_eip1559_transaction(
    tx: Eip1559Transaction,
    pvt_key: [u8; 32],
) -> Result<SignedTransaction, ExecutionError> {
    // ============================================================
    // prepare rlp(unsigned_tx)

    let unsigned_rlp = encode_eip1559_unsigned(&tx)?;

    let mut hasher = Keccak256::new();
    hasher.update([0x02]);
    hasher.update(&unsigned_rlp);
    let signing_hash = hasher.finalize();

    // ============================================================
    // Load key & sign hash (recoverable)

    let signing_key = SigningKey::from_bytes(&pvt_key.into())
        .map_err(|e| ExecutionError::Internal(format!("Invalid private key: {e}")))?;

    let mut pk = pvt_key;
    pk.zeroize();

    let (mut signature, recovery_id): (Signature, RecoveryId) = signing_key
        .sign_prehash_recoverable(&signing_hash)
        .map_err(|e| ExecutionError::Invariant(format!("Signing failed: {e}")))?;

    // ============================================================
    // canonicalize

    if signature.s().is_high().into() {
        signature = signature
            .normalize_s()
            .ok_or_else(|| ExecutionError::Internal("Failed to normalize signature".into()))?;
    }

    // yParity ∈ {0,1}
    let y_parity: u8 = recovery_id.to_byte();

    let r_bytes = signature.r().to_bytes();
    let s_bytes = signature.s().to_bytes();

    // ============================================================
    // package and return signed transaction_rlp

    let signed_rlp = encode_eip1559_signed(&tx, y_parity, &r_bytes.into(), &s_bytes.into())?;

    let mut out = Vec::with_capacity(1 + signed_rlp.len());
    out.push(0x02);
    out.extend_from_slice(&signed_rlp);

    Ok(SignedTransaction {
        rlp: Bytes::from(out),
    })
}

// ============================================================
