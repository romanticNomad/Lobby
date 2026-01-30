use kernel::{
    traits::EthRlpEncode,
    types::{Eip1559Transaction, ExecutionError},
};
use rlp::RlpStream;

// ============================================================

pub fn encode_eip1559_unsigned(tx: &Eip1559Transaction) -> Result<Vec<u8>, ExecutionError> {
    let mut s = RlpStream::new_list(9);

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

pub fn encode_eip1559_signed(
    tx: &Eip1559Transaction,
    y_parity: u8,
    r: &[u8; 32],
    s: &[u8; 32],
) -> Result<Vec<u8>, ExecutionError> {
    let mut srlp = RlpStream::new_list(12);

    // --- core tx fields ---
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

    // --- access list ---
    srlp.begin_list(tx.access_list.len());
    for (addr, keys) in &tx.access_list {
        srlp.begin_list(2);
        addr.eth_rlp_append(&mut srlp);

        srlp.begin_list(keys.len());
        for key in keys {
            key.eth_rlp_append(&mut srlp);
        }
    }

    // --- signature fields (EIP-1559) ---
    srlp.append(&y_parity); // yParity ∈ {0,1}
    srlp.append(&r.as_slice()); // r: 32-byte big-endian
    srlp.append(&s.as_slice()); // s: 32-byte big-endian (LOW-S enforced earlier)

    Ok(srlp.out().to_vec())
}

// ============================================================
