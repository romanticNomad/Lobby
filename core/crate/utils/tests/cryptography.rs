// ============================================================
// checking if the alloy provided 'keccak-256' hash is the one used by evm

use alloy::primitives::Keccak256;
use hex_literal::hex;

#[test]
fn check_evm_keccak_256() {
    let mut hash = Keccak256::new();
    hash.update([]);
    let output = hash.finalize();

    let keccak_expected = hex!("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470");
    assert_eq!(
        output.as_slice(),
        keccak_expected,
        "Hasher is not ethereum Keccak-256"
    );
}

// ============================================================
