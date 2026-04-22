# Lobby

> **Prototype Notice:** Lobby is currently in active development. APIs, features, and behaviours described in this document may change in future releases. Please refer to the [GitHub repository](https://github.com/romanticNomad/Lobby) for the latest updates.
>
> **Do Not** use **Lobby** for transferring real money on EVM accounts. This software is intended for testing and development purposes only.
>
> **Author Note**: This `README.md` is written with the help of an `LLM`, I have corrected any possible loose text to ensure that the instructions and contents remain clear.

---

**Version:** 0.1.0 (Prototype)  
**Last Updated:** April 21 2026  
**Target Audience:** Contributors, LLMs working on the Lobby codebase and EVM transaction operators.

---

## 1. Introduction to Lobby

Lobby is a high-performance, low-latency blockchain transaction service written in `rust` and designed for developers who need reliable, concurrent transaction processing at scale on EVM-compatible networks.

### What Makes Lobby Special

**Sub-second internal processing** — Nonce assignment and signing are typically complete in under one second through optimised actor-based concurrency.

**Concurrent pipeline architecture** — Process thousands of transactions simultaneously via actor-based sharding, with deterministic routing ensuring sequential nonce assignment per address while maintaining parallelism across different accounts.

**Automatic recovery** — Built-in retry logic for transient failures, nonce mismatch detection with automatic sync and re-sign, and stale nonce cleanup via background processes.

**Multi-chain support** — Single API for Ethereum mainnet, Polygon, Arbitrum, Hoodi testnet, and other EVM-compatible networks.

**Real-time status tracking** — Poll transaction progress from submission through on-chain confirmation with detailed status at every pipeline stage.

---

## 2. Quick Installation Guide

### Prerequisites

- Rust 1.70+ with Cargo
- Docker and Docker Compose
- Access to EVM RPC endpoints (Alchemy, Infura, or self-hosted nodes)

### 1. Clone and Setup Infrastructure

```bash
git clone https://github.com/romanticNomad/Lobby.git
cd Lobby

# Start PostgreSQL and Redis
cd database
docker compose up -d
sqlx migrate run
cd ..
```

### 2. Generate Test Keys with Locket

```bash
# exit control from the Lobby workspace
cd ..
# Clone and run Locket for test account generation
git clone https://github.com/romanticNomad/Locket.git
cd Locket
# using 5 as an example, you may use any (positive) number 
cargo run -- accounts 5
# Copy the generated key output
mv accounts.json ../Lobby/test-keys.json
cd ../Lobby
```

`test_keys.json` will be created in the project root (ensure it's in this format):

```json
{
  "test_account": {
    "pvt_key": "0x<64_hex_characters>",
    "pub_key": "0x<128_hex_characters>",
    "address": "0x<40_hex_characters>"
  }
}
```

### 3. Generate test- API keys from the `test-keys.json` using the `generate_api_keys.rs` binary

The `generate_api_keys` binary will automatically create the lobby-formatted API keys
from the `test-keys.json` and store them in the .env file

>**Author Note**:  never store production API keys and EVM private keys in the .env file

```bash
#run the generate_api_keys binary
cargo run --release --bin generate_api_keys
```

### 4. Configure Environment

Create `.env` in the project root:

```bash
export DATABASE_URL="postgresql://lobby:lobby_dev_password@localhost:5432/lobby-db"
export SERVER_ADDR="0.0.0.0:3000"
export REDIS_URL="redis://localhost:6379"
export RUST_LOG="INFO"

# API key format: <token>:<client_id>:<from_address>
export LOBBY_API_KEY_1="<api-key-generated-by-the-binary>"
# There must 1 or more API keys generated, depending on the number `N` you chose.

# RPC endpoints (comma-separated for multiple endpoints per chain)
export RPC_ENDPOINT_1="https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY,https://mainnet.infura.io/v3/YOUR_KEY"
export RPC_ENDPOINT_560048="https://eth-hoodi.g.alchemy.com/v2/YOUR_KEY,https://hoodi.infura.io/v3/YOUR_KEY"
```

### 5. Build and Run

```bash
source .env
cargo run --release --bin lobby
```

Expected startup output:

```bash
INFO     ｉ [info]: "relation \"_sqlx_migrations\" already exists, skipping"
INFO     ｉ [info]: api_keys loaded: 5
INFO     ｉ [info]: custody accounts loaded: 5
INFO     ｉ [info]: rpc_endpoints loaded: {ChainId(560048): 2, ChainId(1): 2}
INFO     ｉ [info]: StatusRegistry loaded | artifacts_count: 0
INFO     ｉ [info]: cortex online
INFO     ｉ [info]: bots spawned: monitoring status
INFO     ｉ [info]: lobby listening at: | address: 0.0.0.0:3000
```

### 6. Test Submission
* For standard test-mainnet gas parameters
* Value = 0.1 eth
* Use API of a valid `address` from the `test-keys.json` for testing.

```bash
curl -X POST http://localhost:3000/v1/transactions \
  -H "Authorization: Bearer <api-key>" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_sendRawTransaction",
    "params": [{
      "from": "<your_address>",
      "to": "<recipient_address>",
      "value": "0x2386f26fc10000",
      "chainId": "0x88bb0",
      "gas": "0x5208",
      "maxFeePerGas": "0xba43b7400",
      "maxPriorityFeePerGas": "0x77359400"
    }],
    "id": 1
  }'
```

---

## Architectural Overview

## 3. Table of Contents

- [3.1 Concurrency Management in Lobby](#31-concurrency-management-in-lobby)
- [3.2 Pipeline Workflow](#32-pipeline-workflow)
- [3.3 RPC Controls and Load Balancing](#33-rpc-controls-and-load-balancing)
- [3.4 Error Handling](#34-error-handling)
- [3.5 Private Key Security](#35-private-key-security)

---

## 3.1 Concurrency Management in Lobby

Lobby employs a multi-layered concurrency model combining the actor pattern, sharding, and database-level atomicity.

### Actor Pattern with MPSC Channels

Every mutable state in Lobby is owned exclusively by a single long-lived Tokio task (the *engine*). External code interacts via typed messages over MPSC channels with oneshot reply channels for request/response patterns.

Each actor follows a consistent three-file structure:

**`mod.rs`** — Public API with spawn function:

```rust
pub fn spawn_nonce_actor(db: PgPool, buffer_size: usize) -> NonceHandle {
    let (tx, rx) = mpsc::channel(buffer_size);
    let engine = NonceEngine::new(db, rx);
    tokio::spawn(async move { engine.run().await });
    NonceHandle::new(tx)
}
```

**`engine.rs`** — Event loop processing commands serially:

```rust
pub async fn run(mut self) {
    while let Some(cmd) = self.rx.recv().await {
        match cmd {
            NonceCommand::Reserve { reply, .. } => {
                let result = self.handle_reserve(...).await;
                let _ = reply.send(result);
            }
        }
    }
}
```

**`handle.rs`** — Cloneable sender implementing traits:

```rust
#[derive(Clone)]
pub struct NonceHandle { tx: mpsc::Sender<NonceCommand> }

#[async_trait]
impl NonceManager for NonceHandle {
    async fn reserve(&self, ...) -> Result<TxNonce, LocalError> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(NonceCommand::Reserve { ..., reply }).await?;
        rx.await?
    }
}
```

This design eliminates data races by construction — no shared mutexes or atomics over actor state.

### Pipeline Semaphore

A `tokio::sync::Semaphore` bounds concurrent pipeline executions (default: 17). If all permits are exhausted, `submit()` blocks for a configurable timeout before returning `BackpressureTimeout` (HTTP 503). This prevents unbounded concurrency from overwhelming the database connection pool, RPC rate limits, or system memory.

### Actor Sharding via ShardPool

Rather than single-actor bottlenecks or fully concurrent access, Lobby runs N sharded actor instances with deterministic routing:

| Actor | Sharding Key | Purpose |
|-------|--------------|---------|
| Nonce | `ByAddress(&from_address)` | Sequential nonce assignment per address |
| Sign | `ByExecutionId(&execution_id)` | Stateless load balancing for ECDSA operations |
| Broadcast | `ByChainId(&chain_id)` | Per-chain RPC state isolation |
| Validator | `ByChainId(&chain_id)` | Per-chain validation isolation |

Routing uses `DefaultHasher`: `shard_index = hash(key) % N`. Same key maps to same actor ensuring sequential ordering; different keys run in true parallel.

### Database-Level Concurrency Safety

PostgreSQL atomic operations with `FOR UPDATE SKIP LOCKED` ensure TOCTOU-safe operations. All insertions use the pattern:

```sql
INSERT INTO table (execution_id, revision, ..., state)
SELECT $1, COALESCE((SELECT MAX(revision) FROM table WHERE execution_id=$1), 0) + 1, ..., 'state'
WHERE NOT EXISTS (
    SELECT 1 FROM table
    WHERE execution_id = $1
      AND (state = 'finalized' OR (state = 'reserved' AND updated_at > now() - interval '2 minutes'))
)
```

Lease-based idempotency provides 2-5 minute windows for operation completion. Revision-based audit trails record every state change immutably.

---

## 3.2 Pipeline Workflow

Every transaction flows through a five-stage pipeline orchestrated by the Cortex:

```
Client Submit
      |
      v
┌─────────────────────────────────────────────────────────────┐
│                    LOBBY PIPELINE                           │
├─────────────────────────────────────────────────────────────┤
│  1. RelayHost        → Validate & persist transaction       │
│  2. Nonce Reserve    → Assign sequential nonce              │
│  3. Sign             → Generate EIP-1559 signature          │
│  4. Broadcast        → Submit to blockchain RPC             │
│  5. Validator        → Confirm on-chain inclusion           │
└─────────────────────────────────────────────────────────────┘
      |
      v
  Confirmed or Failed
```

### Stage Details

**Stage 1 — RelayHost:** Validates transaction fields (gas limits, fee sanity, supported chains) and persists to `relay_host.transaction_intents`. Idempotent by `execution_id`.

**Stage 2 — Nonce Reserve:** Atomically selects next available nonce with three priorities: (1) reuse released nonces, (2) next sequential nonce, (3) zero for new addresses. Single `INSERT ... SELECT ... WHERE NOT EXISTS` query eliminates TOCTOU windows.

**Stage 3 — Sign:** Loads private key from custody, signs using EIP-1559 RLP encoding with secp256k1 ECDSA. Returns `SignedTransaction` with embedded nonce.

**Stage 4 — Broadcast:** Submits signed transaction via RPC. Detects `NonceTooLow` errors and triggers automatic recovery. Uses sticky session endpoint affinity for nonce consistency.

**Stage 5 — Validator:** Polls `eth_getTransactionReceipt` until confirmation or timeout. Finalizes nonce on success; releases or consumes nonce on failure/timeout.

### Async Execution Model

The HTTP handler responds immediately with `execution_id` and `status: "accepted"`. The pipeline runs entirely in a spawned background task, enabling fire-and-forget submission with subsequent status polling via `GET /status/:execution_id`.

### Status Tracking

`StatusRegistry` provides two-layer storage: in-memory `DashMap` for O(1) reads and Redis for distributed persistence. Status transitions include:

- `PermitAcquired` → `Accepted` → `NonceReserved` → `Signed` → `Broadcasted` → `ConfirmedOnChain`
- Transient: `NonceMismatchDetected` (during auto-recovery)
- Terminal failure: `Failed { stage, reason }`
- Terminal timeout: `ValidatorTimedOut`

---

### 3.3 RPC Controls and Load Balancing

The `utils::rpc` module provides high-throughput RPC operations with intelligent load balancing and endpoint health management.

### Dual EndpointPool Architecture

Separate `EndpointPool` instances for Broadcast and Validator actors prevent broadcast traffic from affecting validator polling. Each endpoint is registered to both pools with separate provider instances and metrics for true isolation:

```rust
RpcProviderStack {
    broadcast: EndpointRegistry,
    broadcast_semaphore: Semaphore(32),
    validator: EndpointRegistry,
    validator_semaphore: Semaphore(32),
}
```

Endpoint IDs follow the pattern `<actor>-<chain_id>-<index>` enabling per-actor metric tracking.

### Load Balancing Strategies

**WeightedLeastResponseTime:** Uses a roulette wheel algorithm based on endpoint health and response time:

```
score = (1000.0 / avg_response_time_ms) * traffic_multiplier
```

Health states apply multipliers: `Healthy = 1.0`, `Degraded = 0.5`, `Unhealthy = 0.0`. Selection is O(n) with lock-free metric reads via atomic ring buffers.

**StickySession:** Index-based selection for endpoint affinity. Critical for nonce consistency — reading nonce from the same endpoint used for broadcasting avoids replication lag issues. Falls back to weighted selection if the sticky endpoint is unhealthy.

### Endpoint Health and Circuit Breaker

Three health states tracked per endpoint:

- **Healthy:** Full traffic weight (error rate < 15%)
- **Degraded:** Reduced traffic (error rate 15-40%)
- **Unhealthy:** Zero traffic, circuit breaker active (error rate > 40%)

Circuit breaker activates on consecutive failures with exponential backoff: 10s → 30s → 60s. Auto-resets on successful request after expiry.

Metrics use lock-free atomic operations with 128-sample rolling windows for response times. Health calculations require minimum 10 requests to prevent volatile early readings.

### Generic RPC Execution Functions

**`execute_unary<F, Fut, R>`:** Primary generic execution function:

```rust
pub async fn execute_unary<F, Fut, R>(
    &self,
    actor: SelectActor,           // Broadcast or Validator
    chain_id: ChainId,
    sticky_index: Option<usize>, // Endpoint affinity
    timeout: Duration,
    operation: F,                // Closure taking provider
) -> Result<R, LobbyRpcError>
```

Automatically records success/failure metrics and handles permit acquisition/relinquishment.

**`acquire_unary_context`:** Advanced context API for manual metric recording or sticky session management:

```rust
pub async fn acquire_unary_context(
    &self,
    actor: SelectActor,
    chain_id: &ChainId,
    sticky_index: Option<usize>,
    timeout: Duration,
) -> Result<(UnaryContext, OwnedSemaphorePermit), LobbyRpcError>
```

Returns a `UnaryContext` with methods: `record_success()`, `record_failure()`, `provider()`, `metrics()`.

**`acquire_healthy_endpoint`:** Fetches best endpoint index for sticky session initialization. Uses the Validator pool (highest traffic = most accurate health data):

```rust
pub async fn acquire_healthy_endpoint(
    &self,
    chain_id: ChainId,
    timeout: Duration,
) -> Result<Option<usize>, LobbyRpcError>
```

### High-Level Helper Functions

**`send_raw_transaction`:** Broadcasts signed transactions with configurable load balancing strategy.

**`get_transaction_count`:** Retrieves pending nonce. Sticky session recommended for consistency.

**`get_transaction_receipt`:** Queries transaction receipt for validation.

**`get_block_number`:** Retrieves current block number for confirmation counting.

---

## 3.4 Error Handling

Lobby categorizes errors as terminating (fatal, abort immediately) or non-terminating (transient, eligible for retry).

### Terminating Errors

Terminating errors indicate deterministic failures that will not succeed on retry:

| Error | Cause | Action |
|-------|-------|--------|
| `MissingProvider` | Chain not configured in RPC endpoints | Operator must add configuration |
| `FromAddressMismatch` | Transaction `from` does not match API key bound address | Client must use correct key |
| `ValidationFailure` | Gas limit zero, excessive fees, unsupported chain | Client must fix parameters |
| `InsufficientFunds` | Account balance below transaction value + gas | Fund the account |
| `NonceTooLow` (after recovery) | On-chain nonce permanently ahead after sync attempt | Manual intervention required |

All terminating errors immediately abort the pipeline without further retry attempts.

### Non-Terminating Errors

Non-terminating errors trigger the `retry_with_backoff` mechanism using full-jitter exponential backoff:

```
window = min(max_delay, base_delay * 2^(attempt-1))
sleep = rand(0, window)
```

**RetryDecision Pattern:**

```rust
pub enum RetryDecision<E> {
    Retry(E),              // Transient, backoff and retry
    FailImmediately(E),      // Fatal, abort now
}
```

Operations return `Result<T, RetryDecision<E>>` to signal retry eligibility. Each retry emits a WARN event; final failure emits ERROR.

### NonceTooLow Handling

When broadcast detects `NonceTooLow` (divergence between database and on-chain nonce):

1. **Consume incorrect nonce:** `resolve(execution_id, Consumed)` marks nonce so it is not recycled.
2. **Revert sign state:** `signer.revert(execution_id)` allows re-signing for same `execution_id`.
3. **Sync with on-chain:** `nonce.sync(chain_id, from_address, execution_id, nonce_on_chain)` reserves the authoritative nonce.
4. **Re-sign:** Sign transaction with corrected nonce.
5. **Retry broadcast:** Single retry attempt with newly signed transaction.

A guard variable (`nonce_retry_attempted`) ensures recovery runs at most once per execution, preventing infinite loops.

### Nonce Gaps

Ethereum processes transactions strictly in nonce order. Missing nonces block all higher nonces indefinitely.

**Sweeper Bot:** Runs every 30 seconds to release stale `reserved` nonces (leases > 2 minutes 5 seconds). This handles gaps from crashes between nonce reservation and broadcast.

**Scanner Bot (Deprecated):** Previously responsible for resolving `timed_out` transactions by polling RPC for eventual inclusion. This component is currently deprecated and non-functional. Future releases will include updated gap handling mechanisms.

Nonce cleanup rules by failure stage:

| Failure Stage | Nonce Action |
|---------------|--------------|
| RelayHost or Nonce Reserve | None (never reserved) |
| Sign | `resolve(Released)` |
| Broadcast (hard fail) | `resolve(Released)` |
| Validator NotIncluded | `resolve(Released)` |
| Validator Timeout | `resolve(Consumed)` (not reusable, may still confirm) |
| Validator Included | `resolve(Finalized)` |

---

## 3.5 Private Key Security

### Current Prototype Implementation

Private keys are stored in plaintext in `test_keys.json` and held unencrypted in memory by the `JsonPolicyEngine`. This implementation is appropriate **only for testnet accounts and local development**.

The file is `.gitignore`d but must be manually managed by the operator.

### Planned: AWS Envelope Encryption

Future production versions will implement AWS KMS-backed envelope encryption:

1. **Data Encryption Key (DEK):** Each private key is encrypted at rest using a DEK.
2. **Customer Master Key (CMK):** The DEK itself is encrypted by an AWS KMS CMK.
3. **Boot-time decryption:** Lobby calls AWS KMS `Decrypt` to obtain plaintext DEK, held only in process memory, which is then used to decrypt the private key (in memory) during runtime.
4. **Runtime security:** Plaintext private keys are held in memory for ensuring low-latency signing; all key material is zeroized on shutdown.

**Benefits:**
- Private keys are never stored in plaintext outside the running process
- KMS access is auditable via AWS CloudTrail
- Lobby refuses to start if KMS is unreachable (boot-time security check)
- Key rotation is possible by re-encrypting blobs without code changes

**Note:** AWS Envelope Encryption will not be implemented in the prototype version of Lobby.

---

*Built with Rust, Tokio, Axum, PostgreSQL, and Redis.*  
*Designed for developers who need reliable, low-latency blockchain transaction infrastructure.*
