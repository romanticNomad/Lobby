# Lobby
Personal on-chain trasaction service.

## lobby directory structure.
1) kernel
* houses generic elements that builds up lobby.
   * local -> local traits.
   * io -> io bound traits.

2) cortex
> redis implimentation.
* tracks state.
* allocates and locks nonce.
* hands tx to seal.
* handles possible concurrency / nonce defaults.

3) relay
* serializes RLP tx payload.
* hands tx to cortex for nonce allocation.
* canonicalize and assemble signed tx.
* handles broadcasting.

4) seal
> AWS implimentation.
* authorizes tx from user.
* invokes KMS for signing.
* send the DER encoded tx to relay.

5) main
* Wires down all the crates.
