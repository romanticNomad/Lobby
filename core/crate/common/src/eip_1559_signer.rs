use alloy::primitives::bytes::Bytes;
use k256::{
    ecdsa::{RecoveryId, Signature, SigningKey},
    elliptic_curve::scalar::IsHigh,
};
use kernel::traits::EthRlpEncode;
use kernel::types::{Eip1559Transaction, LocalError, SignedTransaction};
use rlp::RlpStream;
use sha3::{Digest, Keccak256};
use zeroize::Zeroize;

// ============================================================

pub fn sign_eip1559_transaction(
    tx: Eip1559Transaction,
    pvt_key: [u8; 32],
) -> Result<SignedTransaction, LocalError> {
    // ============================================================
    // prepare keccak-256(rlp(unsigned_tx)) for EIP-1159

    let unsigned_rlp = encode_eip1559_unsigned(&tx)?;

    let mut hasher = Keccak256::new();
    hasher.update([0x02]);
    hasher.update(&unsigned_rlp);
    let signing_hash = hasher.finalize();

    // ============================================================
    // load key & produce (signature, recovery_id) -> { signature = sekp256k1( keccak-256(rlp_unsigned_tx), pvt_key ) }

    let signing_key = SigningKey::from_bytes(&pvt_key.into())
        .map_err(|e| LocalError::Internal(format!("Invalid private key: {e}")))?;

    let mut pk = pvt_key;
    pk.zeroize();

    let (mut signature, recovery_id): (Signature, RecoveryId) = signing_key
        .sign_prehash_recoverable(&signing_hash)
        .map_err(|e| LocalError::Invariant(format!("Signing failed: {e}")))?;

    // ============================================================
    // canonicalize to low 's'

    if signature.s().is_high().into() {
        signature = signature
            .normalize_s()
            .ok_or_else(|| LocalError::Internal("Failed to normalize signature".into()))?;
    }

    // yParity ∈ {0,1}
    let y_parity: u8 = recovery_id.to_byte();

    let r_bytes = signature.r().to_bytes();
    let s_bytes = signature.s().to_bytes();

    // ============================================================
    // package into 'Bytes' and return 0x02 || rlp(signed_tx)

    let signed_rlp = encode_eip1559_signed(&tx, y_parity, &r_bytes.into(), &s_bytes.into())?;

    let mut out = Vec::with_capacity(1 + signed_rlp.len());
    out.push(0x02);
    out.extend_from_slice(&signed_rlp);

    Ok(SignedTransaction {
        rlp: Bytes::from(out),
    })
}

// ============================================================
// encoding unsigned tx to rpl stream

pub fn encode_eip1559_unsigned(tx: &Eip1559Transaction) -> Result<Vec<u8>, LocalError> {
    let mut s = RlpStream::new_list(9);

    // core tx fields
    tx.chain_id.eth_rlp_append(&mut s);
    tx.nonce.eth_rlp_append(&mut s);
    tx.max_priority_fee_per_gas.eth_rlp_append(&mut s);
    tx.max_fee_per_gas.eth_rlp_append(&mut s);
    tx.gas_limit.eth_rlp_append(&mut s);

    match &tx.to {
        Some(to) => to.eth_rlp_append(&mut s),
        None => {
            s.append_empty_data();
        }
    }

    tx.value.eth_rlp_append(&mut s);
    s.append(&tx.data);

    // accessList
    s.begin_list(tx.access_list.len());
    for (addr, keys) in &tx.access_list {
        s.begin_list(2);
        addr.eth_rlp_append(&mut s);

        s.begin_list(keys.len());
        for key in keys {
            key.eth_rlp_append(&mut s);
        }
    }

    Ok(s.out().to_vec())
}

// ============================================================
// encode signed transaction to rlp stream

pub fn encode_eip1559_signed(
    tx: &Eip1559Transaction,
    y_parity: u8,
    r: &[u8; 32],
    s: &[u8; 32],
) -> Result<Vec<u8>, LocalError> {
    let mut srlp = RlpStream::new_list(12);

    // core tx field
    tx.chain_id.eth_rlp_append(&mut srlp);
    tx.nonce.eth_rlp_append(&mut srlp);
    tx.max_priority_fee_per_gas.eth_rlp_append(&mut srlp);
    tx.max_fee_per_gas.eth_rlp_append(&mut srlp);
    tx.gas_limit.eth_rlp_append(&mut srlp);

    match &tx.to {
        Some(to) => to.eth_rlp_append(&mut srlp),
        None => {
            srlp.append_empty_data();
        }
    }

    tx.value.eth_rlp_append(&mut srlp);
    srlp.append(&tx.data);

    // access list
    srlp.begin_list(tx.access_list.len());
    for (addr, keys) in &tx.access_list {
        srlp.begin_list(2);
        addr.eth_rlp_append(&mut srlp);

        srlp.begin_list(keys.len());
        for key in keys {
            key.eth_rlp_append(&mut srlp);
        }
    }

    // signature fields (EIP-1559)
    srlp.append(&y_parity); // yParity ∈ {0,1}
    srlp.append(&r.as_slice()); // r: 32-byte big-endian
    srlp.append(&s.as_slice()); // s: 32-byte big-endian (LOW-S)

    Ok(srlp.out().to_vec())
}

// ============================================================
