# Lobby
Personal on-chain trasaction service.

## lobby directory structure.
1) kernel
   * contains signatures to all elements of lobby

2) membrane
   * receives RLP payload
   * validates shape
   * forwards inward
   * broadcasts tx
   * returns response

3) memory
   * allocates nonce
   * locks nonce
   * enriches tx (nonce, gas, chain id)
   * hands tx to signing 
   * tracks state
   * decides next step

4) seal
   * canonicalizes / serializes tx
   * invokes KMS (via aws adapter)
   * assembles signed tx
   * returns signed payload

5) master
   * Wires down all the crates
