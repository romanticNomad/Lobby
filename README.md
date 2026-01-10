# Lobby
Personal on-chain trasaction service.

## lobby directory structure.
1) kernel
* houses generic elements that build up the lobby.
   * local -> local traits.
   * io -> io bound traits.

2) cortex
> redis implementation.
* tracks state.
* allocates and locks nonce.
* hands tx to seal.
* handles possible concurrency / nonce defaults.

3) relay
* serialises RLP tx payload.
* hands tx to cortex for nonce allocation.
* canonicalise and assemble signed tx.
* handles broadcasting.

4) seal
> AWS implementation.
* authorises tx from user.
* invokes KMS for signing.
* hands the DER encoded tx to the relay.

5) main
* wires down all the crates.
