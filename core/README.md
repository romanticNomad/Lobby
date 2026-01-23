## 1. Actors and Their Roles in Lobby

Actors are **long-lived Tokio tasks** that **exclusively own a category of mutable, order-sensitive state** and make **authoritative decisions**. Each actor processes messages sequentially and persists facts to PostgreSQL.

### 1.1 RelayHost (Ingress Actor)

**Responsibility**

* Accept transaction requests from external transports (EIP-1193, WalletConnect, localhost, relays).
* Normalize requests into Lobby `Intent`s.
* Enforce transport-level idempotency and rate limits.

**Owns**

* External request → `ExecutionId` mapping
* Authentication / authorization context (if persisted)

**Does NOT own**

* Nonces, signing, broadcasting, validation

**Interfaces**

* Input: transport adapters (pure translation)
* Output: `Intent` → Pipeline submission

**Persistence**

* PostgreSQL: execution registration
* Redis: ingress buffering, short-TTL dedupe, rate limits

---

### 1.2 NonceManager (Ordering Actor)

**Responsibility**

* Own the nonce timeline for `(chain_id, from)`.
* Serialize nonce reservation, reuse, replacement eligibility, and final resolution.

**Owns**

* Nonce state machine: `available → reserved → inflight → finalized/released`

**Interfaces**

* Command: `ReserveNonce { chain_id, from, execution_id }`
* Command: `ResolveNonce { execution_id, outcome }`
* Event: `NonceReserved`, `NonceFinalized`

**Persistence**

* PostgreSQL: nonce assignments and resolution (authoritative)
* Redis: per-key work queues, wakeups

**Rules**

* No other component may assign or reuse a nonce.
* Idempotency enforced via `ExecutionId`.

---

### 1.3 Signer (Custody Actor)

**Responsibility**

* Authoritatively decide whether a raw transaction may be signed.
* Serialize signing per key and handle AWS KMS / HSM interactions.

**Owns**

* Signing order
* Retry and backpressure policies
* Auditability of signed payloads

**Interfaces**

* Command: `SubmitSigning { execution_id, raw_tx }`
* Event: `TxSigned { execution_id, signed_tx }`

**Persistence**

* PostgreSQL: signing requests, signed artifacts, audit records
* Redis: signing queues, per-key backpressure

**Notes**

* Inline signing is acceptable for prototyping.
* Production implementation MUST be actor-based.

---

### 1.4 Broadcaster (Execution Actor)

**Responsibility**

* Submit signed transactions to the network.
* Manage retries, gas bumping, and replacement transactions.

**Owns**

* Broadcast attempts
* Economic decisions (latency vs cost)
* Replacement tx creation (with NonceManager coordination)

**Interfaces**

* Command: `SubmitBroadcast { execution_id, signed_tx }`
* Event: `TxBroadcasted`, `TxReplaced`

**Persistence**

* PostgreSQL: broadcast attempts, tx hashes, gas params
* Redis: retry timers, broadcast queues

---

### 1.5 Validator (Truth Actor)

**Responsibility**

* Observe chain state and determine final execution outcome.
* Handle confirmations, reorgs, and finality thresholds.

**Owns**

* On-chain truth for an execution

**Interfaces**

* Command: `WatchExecution { execution_id, tx_hash }`
* Event: `ExecutionFinalized { execution_id, success }`

**Persistence**

* PostgreSQL: final outcome, block numbers, reorg records
* Redis: watch scheduling, polling cadence

---