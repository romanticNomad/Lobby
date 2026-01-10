# Lobby
Personal on-chain transaction service.

## lobby directory structure.
1) kernel
* houses generic elements that build up the lobby.
   * local -> local traits.
   * io -> io bound traits.

2) relay
* receive tx-request payload.
* checks on-chain nonce.
* sends tx-signing request.
* broadcast tx.
* handles io error.

3) cortex
> redis implementation.
* tracks state.
* manages nonce and serialises tx.
* DER-parsing and canonicalise tx.
* updates state.
* handles concurrency defaults.

4) seal
> AWS implementation.
* authenticate tx from user via AWS API gateway.
* invokes KMS for signing.
* hands the DER-encoded tx to the relay.
* AWS error handling.

5) state
> sqlx and postgres.
* stores tx states for:
   * nonce validation.
   * tx audits.

6) main
* execution layer that wires down all the crates.