# Lobby Architecture Guide

> * **Prototype Notice:** Lobby is currently in active development. APIs, features, and behaviors described in this document may change in future releases. Please refer to the [GitHub repository](https://github.com/romanticNomad/Lobby) for the latest updates.  
> * **Do Not** use **Lobby** for mainnet transactions.
> * For `test keys` generation the user may use my evm account genration tool **[Locket](https://github.com/romanticNomad/Locket)**, the user can simply get a new account with a `cargo run` command.

> This Doc contains the architectural details of `Lobby` for API-specific documentation (request formats, error codes, rate limits), see [Lobby_API_Doc](docs/Lobby_API_Doc.md).

> For quick `Lobby` bootup guide visit: [Lobby_Bootup](docs/Lobby_Bootup.md).

**Version:** 0.1.0 (Prototype)  
**Last Updates:** March 21 2026  
**Target Audience:** Contributors, maintainers, and LLMs working with the Lobby codebase    

---

## Table of Contents

1. [What is Lobby?](#1-what-is-lobby)
2. [The Actor Model](#2-the-actor-model)
3. [Kernel: Traits and Types](#3-kernel-traits-and-types)
4. [The Five Actors](#4-the-five-actors)
5. [Cortex: The Orchestrator](#5-cortex-the-orchestrator)
6. [Pipeline Workflow](#6-pipeline-workflow)
7. [Retry Mechanics](#7-retry-mechanics)
8. [Concurrency Model](#8-concurrency-model)
9. [Error Handling](#9-error-handling)
10. [Nonce Nuance: NonceTooLow](#10-nonce-nuance-noncetoolow)
11. [Nonce Nuance: Nonce Gaps](#11-nonce-nuance-nonce-gaps)
12. [Background Bots](#12-background-bots)
13. [Utilities](#13-utilities)
14. [Database Architecture](#14-database-architecture)
15. [Status Registry](#15-status-registry)
16. [Security Model](#16-security-model)
17. [Configuration Reference](#17-configuration-reference)
18. [Production Consideration](#18-production-considerations)
19. [API Overview](#19-api-overview)
20. [Appendices](#20-appendices)

---

## 1. What is Lobby?

**Lobby** is a high-performance, low-latency blockchain transaction signing service designed for developers who need reliable, concurrent transaction processing at scale.

### Key Features

- **Sub-second internal processing** — Nonce assignment and signing typically complete in < 1 second
- **Concurrent pipeline architecture** — Process thousands of transactions simultaneously via actor-based sharding
- **Automatic recovery** — Built-in retry logic for transient failures, nonce mismatch detection, and stale nonce cleanup
- **Multi-chain support** — Single API for Ethereum mainnet, Polygon, Arbitrum, and testnets
- **Real-time status tracking** — Poll transaction progress from submission to on-chain confirmation

### Use Cases

- **DApp backends** — Offload transaction signing and nonce management from your application
- **MEV bots** — Low-latency submission for time-sensitive arbitrage and liquidation transactions
- **Batch operations** — Process large volumes of transactions (airdrops, payroll, etc.) with automatic nonce sequencing
- **Multi-chain services** — Unified API for cross-chain transaction submission

### Design Philosophy

Lobby is designed around three core principles:

1. **Reliability over speed** — Every transaction is tracked end-to-end with automatic failure recovery
2. **Transparency** — Detailed status updates at every pipeline stage
3. **Simplicity** — Clean REST API with standard EIP-1193 request format

### Pipeline Stages

Every transaction submitted to Lobby flows through a five-stage pipeline:

```
┌─────────────┐
│   Client    │
│   Submit    │
└──────┬──────┘
       │
       ▼
┌─────────────────────────────────────────────────────────────┐
│                    LOBBY PIPELINE                           │
├─────────────────────────────────────────────────────────────┤
│  1. RelayHost        → Validate & persist transaction       │
│  2. Nonce Reserve    → Assign sequential nonce              │
│  3. Sign             → Generate EIP-1559 signature          │
│  4. Broadcast        → Submit to blockchain RPC             │
│  5. Validator        → Confirm on-chain inclusion           │
└──────┬──────────────────────────────────────────────────────┘
       │
       ▼
┌─────────────┐
│  Confirmed  │
│  or Failed  │
└─────────────┘
```
---

## 2. The Actor Model

Lobby's architecture is built around the **actor pattern**: each piece of mutable state is owned exclusively by a single long-lived Tokio task (the *engine*). External code interacts with it only by sending typed messages over an `mpsc` channel and receiving replies via a `oneshot` channel.

This design eliminates data races by construction — there are no shared mutexes or atomics over state. Two concurrent requests for the same nonce actor are serialized through the queue; they cannot race.

### Anatomy of an Actor

Every actor follows the same three-file structure:

**`engine.rs`** — The event loop. Owns the `PgPool` and `mpsc::Receiver`. Processes one command at a time.

```rust
pub struct NonceEngine { db: PgPool, rx: mpsc::Receiver<NonceCommand> }

impl NonceEngine {
    pub async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd { ... }
        }
    }
}
```

**`handle.rs`** — The public interface. A cheap-to-clone `mpsc::Sender` wrapper that implements the actor's trait from `kernel`.

```rust 
#[derive(Clone)]
pub struct NonceHandle { tx: mpsc::Sender<NonceCommand> }

#[async_trait]
impl NonceManager for NonceHandle {
    async fn reserve(...) -> Result<TxNonce, LocalError> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(NonceCommand::Reserve { ..., reply }).await?;
        rx.await?
    }
}
```

**`mod.rs`** — The spawn function. Creates the channel, spawns the engine task, and returns the handle.

```rust 
pub fn spawn_nonce_actor(db: PgPool, buffer_size: usize) -> NonceHandle {
    let (tx, rx) = mpsc::channel(buffer_size);
    tokio::spawn(async move { NonceEngine::new(db, rx).run().await });
    NonceHandle::new(tx)
}
```

The `actors` crate exposes one public `spawn_*_actor` function per actor. Everything else is internal.

---

## 3. Kernel: Traits and Types

The `kernel` crate contains all shared types, traits, and error enums. It has zero business logic — it is purely a contract definition layer.

### Core Types

| Type | Description |
|---|---|
| `ExecutionId` | Newtype over `uuid::Uuid`. Idempotency key for every pipeline run. |
| `ChainId` | Newtype over `U256`. Identifies the target EVM network. |
| `TxNonce` | Newtype over `U256`. Ethereum transaction nonce. |
| `Eip1559Transaction` | Normalized EIP-1559 transaction struct (chain_id, nonce, gas, fees, to, value, data, access_list). |
| `SignedTransaction` | RLP-encoded signed transaction bytes + the nonce it was signed with. |
| `BroadcastOutcome` | Result of a successful broadcast: `{ txn_hash: TxHash }`. |
| `ClientConfig` | Authenticated client context: `{ client_id: Uuid, from_address: Address }`. |
| `RpcProviderRegistry` | `Arc<DashMap<ChainId, Arc<dyn Provider>>>` — shared RPC provider map. |
| `ApiRegistry` | `Arc<DashMap<ApiToken, ClientConfig>>` — loaded from environment variables. |
| `PipelineStatus` | Enum tracking pipeline progress (see section 15). |

### Actor Traits

These traits are implemented by each actor's handle and are what the orchestrator depends on. The orchestrator never imports actor internals — only these traits.

```rust
// Persist the raw transaction intent to the database.
trait IntentRelay {
    async fn register_transaction(execution_id, txn, client_config) -> Result<(), RelayHostError>;
}

// Manage nonce counters per (chain_id, from_address).
trait NonceManager {
    async fn reserve(chain_id, from_address, execution_id) -> Result<TxNonce, LocalError>;
    async fn resolve(execution_id, state: NonceState)   -> Result<(), LocalError>;
    async fn sync(chain_id, from_address, execution_id, nonce_on_chain) -> Result<TxNonce, LocalError>;
}

// Sign a transaction with the private key for `from_address`.
trait Signer {
    async fn sign(chain_id, from_address, execution_id, txn) -> Result<SignedTransaction, LocalError>;
    async fn revert(execution_id) -> Result<(), LocalError>;
}

// Submit a signed transaction to an RPC node.
trait Broadcaster {
    async fn broadcast(chain_id, from_address, execution_id, txn) -> Result<BroadcastOutcome, BroadcastError>;
}

// Poll for on-chain transaction inclusion.
trait Validator {
    async fn validate(chain_id, execution_id, tx_hash) -> Result<ValidatorOutcome, ValidatorError>;
}

// Read/write pipeline status. Implemented by StatusRegistry.
trait StateStore {
    fn set(execution_id, status: PipelineStatus);
    fn get(execution_id) -> Option<PipelineStatus>;
    fn remove(execution_id);
}
```

### Error Hierarchy

Each stage has its own error type, keeping failure reasons specific:

- `RelayHostError` — validation failure, database error, from-address mismatch
- `LocalError` — shared by Nonce and Sign: database error, invariant violation, rejected
- `BroadcastError` — missing RPC provider, rejection, nonce too low, unexpected RPC error
- `ValidatorError` — RPC error, on-chain revert (status=0), database error, internal
- `CortexError` — wraps all of the above, adds `BackpressureTimeout`

---

## 4. The Five Actors

### RelayHost

**Responsibility:** Accept and durably persist transaction intents before the pipeline begins.

**Database schema:** `relay_host.transaction_intents`

**Operations:**
- `register_transaction(execution_id, txn, client_config)` — validates the transaction fields via `transaction_lint()` (gas limit, fee sanity, supported chains) and inserts a row. Idempotent: if `execution_id` already exists, it returns `Ok(())` immediately.

The `relay_host.transaction_intents` table is **write-once** — a trigger prevents updates. This provides an immutable audit log of every transaction that entered Lobby.

**Supported chains (currently hardcoded in `lint.rs`):** Ethereum Mainnet (1), Goerli (5), Hoodi (560048), Polygon (137), Arbitrum One (42161).

---

### Nonce

**Responsibility:** Assign monotonically increasing nonces per `(chain_id, from_address)` without races.

**Database schema:** `nonce.nonce_assignments`

**Operations:**

`reserve(chain_id, from_address, execution_id)` — Atomically selects the next available nonce and inserts a row with `state='reserved'`. The selection logic has three priorities:

1. **Priority 1 — Reuse a released nonce.** If a previous transaction failed and released its nonce, that nonce is recycled first (fills gaps).
2. **Priority 2 — Next sequential nonce.** `MAX(nonce WHERE state IN ('reserved', 'finalized')) + 1`.
3. **Priority 3 — Zero.** Used when no records exist for this address.

The entire selection and insertion is a single `INSERT ... SELECT ... WHERE NOT EXISTS` query — no separate read step, so there is no TOCTOU window.

`resolve(execution_id, state)` — Transitions the most recent `'reserved'` row for an `execution_id` to one of:
- `'finalized'` — transaction confirmed on-chain, nonce is consumed forever
- `'released'` — transaction failed, nonce is available for reuse by Priority 1
- `'consumed'` — transaction broadcast but validator timed out; nonce is not recycled (gap handling, see section 11)

`sync(chain_id, from_address, execution_id, nonce_on_chain)` — Used during nonce mismatch recovery. Reserves the authoritative on-chain nonce directly (see section 10).

---

### Sign

**Responsibility:** Cryptographically sign `Eip1559Transaction` structs.

**Database schema:** `sign.sign_requests`

**Operations:**

`sign(chain_id, from_address, execution_id, txn)` — Loads the private key for `from_address` from the `JsonPolicyEngine`, signs the transaction using EIP-1559 RLP encoding + secp256k1 ECDSA (via `k256`), records the result, and returns `SignedTransaction { rlp, with_nonce }`.

The signing implementation (`utils::eip1559::sign`) encodes the transaction to RLP, hashes with `keccak256(0x02 || rlp(unsigned_tx))`, and produces a low-S canonical signature. The signed transaction is prefixed with type byte `0x02`.

`revert(execution_id)` — Transitions the most recent `'signed'` row to `'failed'`. Called during nonce mismatch recovery so the Sign actor can accept a new `sign()` call for the same `execution_id`.

---

### Broadcast

**Responsibility:** Submit signed transactions to blockchain RPC nodes and detect nonce errors.

**Database schema:** `broadcast.broadcast_requests`

**Operations:**

`broadcast(chain_id, from_address, execution_id, txn)` — Fetches the provider from `RpcProviderRegistry`, calls `send_raw_transaction(&rlp)`, and records the result. Three outcomes:

- **Success** — records `state='submitted'` with `tx_hash`, returns `BroadcastOutcome`.
- **Nonce error** — detects error strings containing `"nonce"` or `"known transaction"`, queries `eth_getTransactionCount(..., pending)` for the authoritative on-chain nonce, records `state='rejected'`, and returns `BroadcastError::NonceTooLow { nonce_on_chain, attempted_nonce }`.
- **Other error** — records `state='rejected'` with the error string, returns `BroadcastError::Rejected`.

Idempotency: if a `state='submitted'` row already exists for the `execution_id`, the broadcast is skipped and the existing `tx_hash` is returned.

---

### Validator

**Responsibility:** Poll RPC nodes for transaction inclusion and confirm the required number of block confirmations.

**Database schema:** `validator.validation_requests`

**Configuration (`ValidatorConfig`):**

| Field | Default | Description |
|---|---|---|
| `poll_interval` | 2 seconds | How often to call `eth_getTransactionReceipt` |
| `timeout` | 300 seconds | Max time before returning `Timeout` |
| `required_confirmations` | 0 | Blocks past inclusion before `Included` is returned |

**Operations:**

`validate(chain_id, execution_id, tx_hash)` — Checks the database for a cached result first (idempotency). If none, records a `state='pending'` row and enters a polling loop:

- No receipt → transaction still pending, sleep `poll_interval`, repeat
- Receipt with `status=0` → records `'not_included'`, returns `ValidatorError::Reverted`
- Receipt with `status=1` → checks `current_block - tx_block >= required_confirmations`, then records `'included'` and returns `ValidatorOutcome::Included`
- Loop timeout → records `'timed_out'`, returns `ValidatorOutcome::Timeout`

---

## 5. Cortex: The Orchestrator

The `cortex` crate wires all five actors together. Its public interface is `CortextHandle`, an `Arc`-backed cheap clone placed in Axum's `AppState`.

### Boot Sequence (`spawn_cortex`)

`spawn_cortex(db, provider, config)` is called once at startup. It:

1. Spawns `config.nonce_shards` Nonce actor instances and wraps them in a `ShardPool`
2. Spawns `config.sign_shards` Sign actor instances in a `ShardPool`
3. Spawns `config.broadcast_shards` Broadcast actor instances in a `ShardPool`
4. Spawns one RelayHost actor
5. Spawns one Validator actor
6. Creates a `Semaphore` with `config.pipeline_concurrency` permits
7. Connects to Redis and initializes `StatusRegistry` (loading any existing entries)
8. Returns `CortextHandle { inner: Arc<Cortex> }`

### ShardPool

`ShardPool<T>` holds a fixed `Vec<Arc<T>>` of actor handles. Routing uses `DefaultHasher`:

```
shard_index = hash(key) % N
```

Three routing key newtypes make call-sites explicit about which dimension is being hashed:

- `ByAddress(&from_address)` → routes to Nonce actors (same address → same actor → sequential)
- `ByExecutionId(&execution_id)` → routes to Sign actors (load balancing, signing is stateless)
- `ByChainId(&chain_id)` → routes to Broadcast actors (same chain → same actor)

### submit()

`cortex_handle.submit(execution_id, txn, client_config)` is the only public method:

1. Acquires a semaphore permit (blocks up to `pipeline_semaphore_timeout` — returns `BackpressureTimeout` if exhausted)
2. Sets `StatusRegistry` entry to `PermitAcquired`
3. Builds a `PipelineContext` bundling all actor handles, config, and status registry
4. Spawns a detached `tokio::spawn` task that calls `run_pipeline(ctx)` and drops the permit on completion
5. Returns `Ok(())` immediately

The HTTP handler then responds to the client with `{ execution_id, status: "accepted" }`. The pipeline runs entirely in the background.

---

## 6. Pipeline Workflow

`run_pipeline(ctx)` runs the five stages sequentially inside a tracing span tagged with `execution_id`, `chain_id`, and `from_address`.

```
submit_transaction (HTTP handler)
  └─ cortex.submit()
       └─ spawn pipeline task
            ├─ RelayHost.register_transaction()   [retried]
            ├─ Nonce.reserve()                    [retried]
            ├─ Sign.sign()                        [retried; releases nonce on hard-fail]
            ├─ Broadcast.broadcast()              [retried; releases nonce on hard-fail]
            │    └─ NonceTooLow? → sync & re-sign & retry once
            └─ Validator.validate()               [releases or finalizes nonce]
```

**Stage 1 — RelayHost:** Calls `register_transaction`. On failure after retries → sets status `Failed`, exits. No nonce was reserved so no cleanup is needed.

**Stage 2 — Nonce:** Gets shard via `ByAddress(&from_address)`, calls `reserve`. On success, stamps `txn.nonce = reserved_nonce` and sets status `NonceReserved`. On failure → sets status `Failed`, exits.

**Stage 3 — Sign:** Gets shard via `ByExecutionId(&execution_id)`, calls `sign(chain_id, from_address, execution_id, txn)`. On success, sets status `Signed`. On failure → calls `release_nonce`, sets status `Failed`, exits.

**Stage 4 — Broadcast:** Gets shard via `ByChainId(&chain_id)`, enters a loop calling `broadcast` with retries. Normal success sets status `Broadcasted { tx_hash }` and proceeds. `NonceTooLow` triggers the one-time recovery path (see section 10). `MissingProvider` is an immediate fatal failure. All other errors after retries → release nonce, set status `Failed`, exit.

**Stage 5 — Validator:** Calls `validate(chain_id, execution_id, tx_hash)`:

- `Included` → calls `finalise_nonce` (resolve to `'finalized'`), sets status `ConfirmedOnChain { tx_hash }`
- `NotIncluded` → calls `release_nonce`, sets status `Failed`
- `Timeout` → calls `consume_nonce` (resolve to `'consumed'`), sets status `ValidatorTimedOut`

The distinction between `release` and `consume` on timeout is intentional — a timed-out transaction may still be in the mempool, so the nonce is not freed for reuse (doing so would create a gap that blocks the mempool sequence). The scanner bot (section 12) later resolves timed-out transactions.

---

## 7. Retry Mechanics

Every pipeline stage is wrapped in `retry_with_backoff(config, stage_label, operation)`.

### Configuration (`RetryConfig`)

| Field | Default | Description |
|---|---|---|
| `max_attempts` | 2 | Retries after the first attempt (total = 3 tries) |
| `base_delay` | 100ms | Initial backoff window |
| `max_delay` | 200ms | Maximum backoff window |

### Backoff Formula

**Full-jitter exponential backoff:**
```
window = min(max_delay, base_delay * 2^(attempt-1))
sleep  = rand(0, window)
```

Full jitter scatters retries across the entire window rather than spiking them at a fixed interval, which prevents a thundering-herd re-spike when many pipelines fail simultaneously (e.g., a brief RPC outage or database hiccup).

### RetryDecision

Operations return `Result<T, RetryDecision<E>>` rather than a plain `Result`. This lets operations signal whether an error is worth retrying:

- `RetryDecision::Retry(e)` — transient, will retry with backoff
- `RetryDecision::FailImmediately(e)` — deterministic, abort immediately without sleeping

Example from the broadcast stage:
```
Err(BroadcastError::NonceTooLow { .. }) => Err(RetryDecision::FailImmediately(..)),
Err(BroadcastError::MissingProvider { .. }) => Err(RetryDecision::FailImmediately(..)),
Err(e) => Err(RetryDecision::Retry(e)),
```

### Logging

- Each retry emits a `WARN` event with `stage`, `attempt`, `delay_ms`, and the error.
- Final failure emits an `ERROR` event.
- Immediate failures emit a `DEBUG` event (no retry attempted).

---

## 8. Concurrency Model

Lobby has two layers of concurrency control.

### Layer 1: Pipeline Semaphore

A `tokio::sync::Semaphore` with `pipeline_concurrency` permits (default: 17) bounds how many pipeline tasks run simultaneously. If all permits are taken, `submit()` blocks for up to `pipeline_semaphore_timeout` ms before returning `CortexError::BackpressureTimeout` (surfaces as HTTP 503 to the client).

This prevents unbounded concurrency from overwhelming the database connection pool, RPC rate limits, or system memory.

### Layer 2: Actor Sharding

Rather than a single actor (bottleneck) or fully concurrent access (races), Lobby runs N sharded actors. The shard count is configurable per actor type.

**Nonce sharding by `from_address`:**
All nonce operations for the same address always route to the same actor. This means nonce assignments for a given address are processed strictly sequentially — no two concurrent pipelines can race on the same nonce counter.

Different addresses hash to (usually) different shards and run in true parallel.

**Sign sharding by `execution_id`:**
Signing is stateless (same private key regardless of shard), so `execution_id` is used purely for load distribution across CPU-bound ECDSA operations.

**Broadcast sharding by `chain_id`:**
All broadcasts for the same chain route to the same actor, allowing per-chain RPC state (connection pools, rate limit tracking) to remain local to one task.

**RelayHost and Validator are not sharded.** RelayHost is purely a DB write with no contention. Validator is I/O-bound (polling RPC); its work is inherently parallel via async tasks inside the engine.

---

## 9. Error Handling

### CortexError

`CortexError` is the top-level error enum for the orchestrator. It wraps all actor errors and adds:

- `BackpressureTimeout { timeout_ms }` — semaphore exhausted
- `NonceReservation(LocalError)` — nonce reserve failed after retries
- `NonceResolve(LocalError)` — nonce resolve failed (non-fatal, lease expires in 5 min)
- `NonceSync(LocalError)` — nonce sync failed (fatal: DB and on-chain are diverged)
- `Sign(LocalError)` / `ReSign(LocalError)` — sign failed before/after nonce sync
- `Broadcast(BroadcastError)` — wraps broadcast failures including `NonceTooLow`
- `NotIncluded(ValidatorError)` — validator confirmed exclusion

`CortexError::stage()` returns a string label (`"relay_host"`, `"nonce_reserve"`, etc.) used in structured logs and the `PipelineStatus::Failed { stage, reason }` payload.

`CortexError::is_transient()` returns `true` only for `BackpressureTimeout`, signaling the HTTP layer to respond with 429/503.

### Nonce Cleanup on Failure

The pipeline has deterministic cleanup rules:

| Failure Stage | Nonce Action |
|---|---|
| RelayHost or Nonce Reserve | None (nonce was never reserved) |
| Sign | `resolve(Released)` |
| Broadcast (hard fail) | `resolve(Released)` |
| Broadcast (NonceTooLow, after sync) | `resolve(Released)` |
| Validator NotIncluded | `resolve(Released)` |
| Validator Timeout | `resolve(Consumed)` |
| Validator Included | `resolve(Finalized)` |

Nonce resolution itself uses `retry_with_backoff`. If resolution fails after all retries, it is logged as an `ERROR` but is **not fatal** — the 5-minute lease on the `'reserved'` state means the Sweeper bot will release it automatically (see section 11).

---

## 10. Nonce Nuance: NonceTooLow

### When It Happens

Lobby maintains its own nonce counter in PostgreSQL. This counter can diverge from the on-chain state when:
- The same address is used outside of Lobby (manual transactions, another service)
- Lobby is first used on an address that already has transaction history
- A chain reorganization invalidates previously confirmed transactions

When the Broadcast actor submits a transaction with a nonce that is already consumed on-chain, the RPC node returns an error containing `"nonce"` or `"known transaction"`. This is `BroadcastError::NonceTooLow`.

### Recovery Sequence

The pipeline handles `NonceTooLow` with a single one-time recovery. A guard variable (`nonce_retry_attempted`) ensures this runs at most once per execution.

**Step 1 — Consume the incorrect nonce**

`resolve(execution_id, Consumed)` marks the incorrectly reserved nonce so it is not recycled (it would immediately fail again if reused).

**Step 2 — Revert sign state**

`signer.revert(execution_id)` transitions the previous `'signed'` DB row to `'failed'`, allowing the Sign actor to accept a new `sign()` call for the same `execution_id` without violating the unique index.

**Step 3 — Sync with on-chain nonce**

`nonce.sync(chain_id, from_address, execution_id, nonce_on_chain)` is called with the authoritative nonce the Broadcast actor fetched from the RPC. The Nonce engine performs a two-part atomic operation:

- It inserts a new `'reserved'` row for `execution_id` at `nonce_on_chain`, provided this execution doesn't already have a valid reservation and no other execution has already reserved that exact nonce.
- If either condition blocks the insert, it returns the existing reservation (idempotency) or `LocalError::Rejected` (another pipeline grabbed the same nonce — pipeline fails loudly).

**Step 4 — Re-sign with corrected nonce**

`txn.nonce = corrected_nonce`, then `sign(...)` again. This produces a new `SignedTransaction` with the correct nonce embedded in the RLP bytes.

**Step 5 — Retry broadcast**

The outer broadcast loop continues with the newly signed transaction. If this second broadcast also fails with `NonceTooLow`, the pipeline hard-fails (the guard variable blocks further recovery). This prevents infinite loops in cases of heavy external contention or misconfiguration.

### StatusRegistry During Recovery

```
NonceReserved → NonceMismatchDetected { nonce_on_chain, attempted_nonce }
             → NonceReserved → Signed → Broadcasted → ConfirmedOnChain
```

Clients polling `/status/:id` will see `NonceMismatchDetected` briefly during recovery. This is informational — the pipeline continues automatically.

---

## 11. Nonce Nuance: Nonce Gaps

### The Problem

Ethereum processes transactions strictly in nonce order. If nonce N is missing while N+1, N+2 are in the mempool, all higher nonces are stuck indefinitely.

Gaps can form when:
- A pipeline crashes between nonce reservation and broadcast
- The database connection fails during `resolve(Released)` after a broadcast error
- The Validator times out on a transaction that eventually gets included (nonce is `'consumed'`, not `'reserved'`)

### Resolution: Sweeper Bot

The Sweeper bot (see section 12) runs every 30 seconds and queries for `'reserved'` nonce rows whose `updated_at` is older than 2 minutes and 5 seconds (the pipeline's max lease duration + a grace buffer):

```sql
UPDATE nonce.nonce_assignments
SET state = 'released'
WHERE (execution_id, revision) IN (
    SELECT DISTINCT ON (execution_id) execution_id, revision
    FROM nonce.nonce_assignments
    WHERE state = 'reserved'
      AND updated_at < now() - interval '2 minutes 5 seconds'
    ORDER BY execution_id, revision DESC
    LIMIT 100
)
RETURNING nonce, execution_id
```

Once a stale `'reserved'` nonce is flipped to `'released'`, it becomes available to the Nonce actor's Priority 1 allocation path. The next incoming transaction for that address will be assigned the gap nonce, which unblocks the on-chain mempool queue automatically when it confirms.

### `Consumed` vs `Released`

The `'consumed'` state (used on validator timeout) is intentionally excluded from the Sweeper. A timed-out transaction may still be in the mempool waiting for a gap to clear. Reusing a `'consumed'` nonce would create a double-spend attempt. Instead, the Scanner bot (section 12) monitors timed-out transactions and resolves their nonces once on-chain inclusion is confirmed or definitively rejected.

---

## 12. Background Bots

Two background tasks spawn during Lobby boot and run for the lifetime of the process.

### Sweeper Bot (`lobby/src/bots/sweeper.rs`)

**Purpose:** Release stale `'reserved'` nonce leases to prevent permanent nonce gaps.

**Interval:** Every 30 seconds.

**Logic:** Finds up to 100 `nonce.nonce_assignments` rows where `state = 'reserved'` and `updated_at < now() - interval '2 minutes 5 seconds'`, transitions them to `'released'`, and logs the count with their `execution_id`s.

If the database is unavailable, the sweeper logs the error and retries on the next tick. It does not crash the process.

Spawned in `main.rs` after migrations:
```rust 
spawn_sweeper_bot(db_pool.clone());
spawn_scanner_bot(
    db_pool.clone(),
    status_registry.clone(),
    rpc_registry.clone(),
);
```

---

### Scanner Bot (`lobby/src/bots/scanner.rs`)

**Purpose:** Resolve `'timed_out'` transactions by polling RPC nodes for eventual inclusion.

**Interval:** Every 30 seconds.

**Logic:**
1. Queries `validator.validation_requests` for up to 100 distinct `execution_id`s with `state = 'timed_out'` (latest revision per execution).
2. Groups them by `chain_id` for concurrent processing.
3. For each transaction, calls `get_transaction_receipt(tx_hash)` on the appropriate RPC provider:
   - Receipt found + `status=1` → updates DB to `'included'`, sets `StatusRegistry` to `ConfirmedOnChain`
   - Receipt found + `status=0` → updates DB to `'not_included'`, sets `StatusRegistry` to `Failed`
   - No receipt → transaction not yet mined, leaves as `'timed_out'` and retries next tick
4. RPC errors are logged per-transaction and do not abort the rest of the batch.

Timed-out transactions that eventually get included via the Scanner Bot do **not** trigger nonce release — their nonces are already in `'consumed'` state, which is correct (the nonce was used on-chain).

Spawned in `main.rs` after the cortex:
```rust
spawn_scanner_bot(db_pool.clone(), status_registry.clone(), rpc_registry.clone());
```

---

## 13. Utilities

The `utils` crate provides three modules consumed by the actors and the HTTP server.

### `utils::eip1559`

**`normalize.rs`** — Converts an `Eip1193SendTransactionParams` (the raw JSON-RPC params from the client) into a `(Eip1559Transaction, Address)` pair. Parses all hex-encoded fields, defaults missing optional fields to zero, and decodes access lists.

**`lint.rs`** — Stateless business logic validation called by RelayHost before persisting:
- Gas limit must be non-zero and ≤ 30,000,000
- `max_fee_per_gas` must be non-zero
- `max_priority_fee_per_gas` must be strictly less than `max_fee_per_gas`
- `chain_id` must be in the supported list (1, 5, 560048, 137, 42161)

**`sign.rs`** — EIP-1559 transaction signing:
1. RLP-encodes the unsigned transaction (9-field list)
2. Hashes as `keccak256(0x02 || rlp)`
3. Signs with secp256k1 ECDSA (`k256` crate), normalizes to low-S
4. RLP-encodes the signed transaction (12-field list including `yParity`, `r`, `s`)
5. Prepends type byte `0x02`
6. Private key bytes are zeroized after use via the `zeroize` crate

### `utils::custody`

**`json_test_policy.rs`** — Implements the `PolicyEngine` trait for the prototype. Loads a `test_keys.json` file from the repository root, builds an in-memory `HashMap<Address, [u8; 32]>` of private keys, and resolves keys by address.

```json
{
  "account1": {
    "pvt_key": "0x...",
    "pub_key":  "0x...",
    "address":  "0x..."
  }
}
```

**Security note:** This is a plaintext key file suitable for testnet use only. See section 16 for the planned production key management model.

### `utils::registry`

**`api.rs`** — `load_api_key_from_env()` scans environment variables matching `LOBBY_API_KEY_<N>=<token>:<client_id>:<from_address>` and builds the `ApiRegistry` DashMap. Panics at boot if no valid keys are found.

**`rpc.rs`** — `load_rpc_endpoints_from_env()` scans `RPC_ENDPOINT_<chain_id>=<url>` environment variables and builds the `RpcProviderRegistry` using Alloy's `ProviderBuilder`. Also provides `get_transaction_receipt()` and `get_block_number()` helpers used by the Validator and Broadcast actors.

---

## 14. Database Architecture

Lobby uses PostgreSQL with SQLx compile-time checked queries (`sqlx::query!`). Migrations live in `database/migrations/` and run automatically at boot via `sqlx::migrate!`.

### Schema Isolation

Each actor owns one schema. There are no foreign keys or cross-schema queries.

| Schema | Table | Owner |
|---|---|---|
| `relay_host` | `transaction_intents` | RelayHost actor |
| `nonce` | `nonce_assignments` | Nonce actor |
| `sign` | `sign_requests` | Sign actor |
| `broadcast` | `broadcast_requests` | Broadcast actor |
| `validator` | `validation_requests` | Validator actor |

### Revision-Based State Tracking

All tables (except `relay_host.transaction_intents`) use `(execution_id BYTEA, revision BIGINT)` as the composite primary key. Current state = the row with the highest revision.

This provides a full audit trail of every state transition for every execution. No row is ever overwritten.

### TOCTOU-Safe Atomic Queries

All insertions use the pattern:
```sql
INSERT INTO <table> (execution_id, revision, ..., state)
SELECT $1, COALESCE((SELECT MAX(revision) FROM <table> WHERE execution_id=$1), 0) + 1, ..., 'initial_state'
WHERE NOT EXISTS (
    SELECT 1 FROM <table>
    WHERE execution_id = $1
      AND (state = '<terminal_state>'
           OR (state = '<active_state>' AND updated_at > now() - interval '<lease>'))
)
RETURNING revision
```

The `WHERE NOT EXISTS` condition and the `INSERT` execute as a single database operation, making it impossible for two concurrent requests to insert duplicate active rows for the same `execution_id`.

### Partial Unique Indexes

Each table enforces uniqueness over active rows:

```sql
-- nonce: only one active reservation per (chain, address, nonce)
CREATE UNIQUE INDEX uniq_active_nonce
ON nonce.nonce_assignments (chain_id, from_address, nonce)
WHERE state IN ('reserved', 'finalized');

-- broadcast: only one in-flight broadcast per execution
CREATE UNIQUE INDEX uniq_active_broadcast
ON broadcast.broadcast_requests (execution_id)
WHERE state IN ('received', 'submitted');
```

### `updated_at` Triggers

Every mutable table has a trigger that updates `updated_at` whenever `state` changes. This timestamp drives lease expiration logic in all `WHERE NOT EXISTS` clauses and in the Sweeper bot.

### Docker Compose

The `database/docker-compose.yml` provides a local development environment:
- PostgreSQL 18.3 on port 5432 (database: `lobby-db`, user: `lobby`)
- Redis 8.6 (AOF persistence) on port 6379
- RedisInsight GUI on port 5540

---

## 15. Status Registry

`StatusRegistry` tracks the current state of every in-flight or recently completed pipeline.

### Storage

It is backed by two layers:
- **DashMap** (`Arc<DashMap<ExecutionId, PipelineStatus>>`) — in-memory, O(1) reads, no network I/O
- **Redis** — distributed persistence via `ConnectionManager`, written asynchronously after every DashMap update

On boot, `StatusRegistry::new(redis_url)` scans all `lobby:status:*` keys from Redis and loads them into the DashMap (crash recovery). New entries are written to both on every `set()` call, with a 1-hour TTL on the Redis key.

Read operations (`get()`) only touch the DashMap — no Redis round-trip.

### PipelineStatus Variants

```rust
pub enum PipelineStatus {
    PermitAcquired,
    Accepted,
    NonceReserved,
    Signed,
    Broadcasted { tx_hash: String, messege: String },
    NonceMismatchDetected { nonce_on_chain: TxNonce, attempted_nonce: TxNonce },
    ConfirmedOnChain { tx_hash: String },
    ValidatorTimedOut { message: String },
    Failed { stage: String, reason: String },
}
```
### Status State Machine

```
                    ┌──────────────────┐
                    │  permit_acquired │
                    └────────┬─────────┘
                             │
                             ▼
                    ┌──────────────────┐
                    │     accepted     │
                    └────────┬─────────┘
                             │
                             ▼
                    ┌──────────────────┐
                    │  nonce_reserved  │
                    └────────┬─────────┘
                             │
                             ▼
                    ┌──────────────────┐
                    │      signed      │
                    └────────┬─────────┘
                             │
                             ▼
              ┌──────────────┴─────────────┐
              │                            │
              ▼                            ▼
  ┌──────────────────────┐      ┌──────────────────────┐
  │ nonce_mismatch_      │      │     broadcasted      │
  │     detected         │      │                      │
  └──────────┬───────────┘      └──────────┬───────────┘
             │                             │
             │ (auto-recovery              │
             │     steps)                  │
             │                             ▼
             └─────────────────► ┌──────────────────────┐
                                 │  confirmed_on_chain  │ ✓
                                 └──────────────────────┘
                                           │
                         ┌─────────────────┴─────────────────┐
                         │                                   │
                         ▼                                   ▼
                ┌────────────────┐                     ┌──────────────┐
                │    failed      │                     │   Validator  │
                │ node-rejection │                     │   timed-out  │
                └────────────────┘                     └──────────────┘
                       ✗                                      ⏳
```
---

## 16. Security Model

### Current Prototype

> Private keys are stored in plaintext in `test_keys.json` and held unencrypted in memory by the `JsonPolicyEngine`. **This is only appropriate for testnet accounts and local development.** The file is `.gitignore`d but must be manually managed by the developer.

API keys are stored in environment variables in plaintext. There is no rotation, expiry, or encryption.

### Planned: AWS Envelope Encryption

The production key management model uses AWS KMS-backed envelope encryption:

1. Each private key is encrypted at rest using a **data encryption key (DEK)**.
2. The DEK itself is encrypted by an **AWS KMS customer master key (CMK)**.
3. At Lobby boot time:
   - Lobby calls AWS KMS (`Decrypt`) to obtain the plaintext DEK.
   - The DEK is held in process memory only (never written to disk).
   - Encrypted private key blobs are decrypted using the in-memory DEK.
   - Plaintext private keys are held in memory for low-latency signing.
4. On shutdown, all key material is zeroized in memory.

**Benefits:**
- Private keys are never stored in plaintext outside the running process
- KMS access is auditable via AWS CloudTrail
- Lobby refuses to start if KMS is unreachable (boot-time security check)
- Key rotation is possible by re-encrypting blobs with a new DEK without changing the signing code

Until this is implemented, Lobby must only be used with testnet private keys.

### Authentication Flow

Incoming requests are authenticated by `auth_middleware` in `lobby/src/server/auth.rs`:

1. Extracts the `Authorization: Bearer <token>` header
2. The token format is `<api_token>:<client_id>:<from_address>`
3. Looks up `api_token` in `ApiRegistry` (DashMap, O(1))
4. On success, attaches `AuthenticatedClient(ClientConfig)` as an Axum extension on the request
5. The `submit_transaction` handler verifies that the `from` field in the JSON-RPC params matches `client_config.from_address` — a mismatch returns HTTP 403

---

## 17. Configuration Reference

All configuration is loaded from environment variables at startup.

### Required

| Variable | Example | Description |
|---|---|---|
| `DATABASE_URL` | `postgres://lobby:lobby_dev_password@localhost:5432/lobby-db` | PostgreSQL connection string |

### Optional (with defaults)

| Variable | Default | Description |
|---|---|---|
| `SERVER_ADDR` | `0.0.0.0:3000` | Axum bind address |
| `REDIS_URL` | `redis://localhost:6379` | Redis connection string for StatusRegistry |
| `NONCE_SHARDS` | `17` | Number of Nonce actor instances |
| `SIGN_SHARDS` | `17` | Number of Sign actor instances |
| `BROADCAST_SHARDS` | `17` | Number of Broadcast actor instances |
| `ACTOR_BUFFER_SIZE` | `64` | mpsc channel buffer per actor |
| `PIPELINE_CONCURRENCY` | `17` | Max concurrent pipeline tasks (semaphore) |
| `PIPELINE_SEMAPHORE_TIMEOUT_MS` | `5000` | Time to wait for a semaphore permit before returning 503 |

### API Keys

Format: `LOBBY_API_KEY_<N>=<api_token>:<client_id>:<from_address>`

- `<N>` is any suffix (1, 2, abc — used for log readability only)
- `<api_token>` is the Bearer token the client sends
- `<client_id>` is a UUID v4 identifying the client
- `<from_address>` is the Ethereum address this client is authorized to transact from

```bash
LOBBY_API_KEY_1=lobby_live_abc123:550e8400-e29b-41d4-a716-446655440000:<from_address>
```

### RPC Endpoints

Format: `RPC_ENDPOINT_<chain_id>=<url>`

```bash
RPC_ENDPOINT_1=https://eth-mainnet.g.alchemy.com/v2/KEY
RPC_ENDPOINT_137=https://polygon-mainnet.g.alchemy.com/v2/KEY
RPC_ENDPOINT_42161=https://arb-mainnet.g.alchemy.com/v2/KEY
RPC_ENDPOINT_560048=https://eth-hoodi.g.alchemy.com/v2/KEY
```

If no RPC endpoints are found, Lobby starts with a warning and all broadcast/validator operations will fail.

### Test Keys (Prototype Only)

Place a `test_keys.json` in the project root:

```json
{
  "account1": {
    "pvt_key": "0x<64_hex_chars>",
    "pub_key":  "0x<128_hex_chars>",
    "address":  "0x<40_hex_chars>"
  }
}
```

The `from_address` in an API key must correspond to an address in `test_keys.json`, or the Sign actor will return a `PolicyEngine` error when it tries to resolve the key.

---

## 18. Production Considerations

### 18.1 Security Best Practices

**API Key Management:**
- **Never commit API keys to version control** — Use environment variables or secret management services (AWS Secrets Manager, HashiCorp Vault, etc.)
- **Rotate keys periodically** — Lobby operators should provide key rotation mechanisms
- **Use separate keys per environment** — Different keys for dev, staging, and production
- **Monitor key usage** — Track which keys are making requests (future Lobby feature)

**Private Key Custody:**

> ⚠️ **Critical:** The current `JsonPolicyEngine` stores private keys in plaintext JSON files. This is **acceptable only for local development**.

Production deployments **must** use:
- **Hardware Security Modules (HSM)** — Dedicated cryptographic hardware
- **Cloud KMS** — AWS KMS, Google Cloud KMS, Azure Key Vault
- **Secure enclaves** — Intel SGX, ARM TrustZone

Future Lobby versions will support pluggable key custody backends.

---

### 18.2 Rate Limiting & Backpressure

**Pipeline Semaphore:**

Lobby enforces concurrency limits via a semaphore (default: 17 concurrent pipelines). If this limit is exceeded:

```json
{
  "error": {
    "code": -32603,
    "message": "pipeline semaphore timed out after 5000ms — server is overloaded"
  }
}
```

**Client-side response:**
- Return `HTTP 429 Too Many Requests` to your end users
- Implement exponential backoff before retrying
- Consider implementing client-side request queuing

**Production tuning:**

Adjust semaphore size via environment variable:
```bash
PIPELINE_CONCURRENCY=50  # Allow 50 concurrent transactions
```

**Capacity planning:**

Lobby's throughput depends on:
- **RPC provider limits** — Alchemy/Infura typically cap at 10-50 req/s per tier
- **Database connections** — PostgreSQL connection pool (default: 17)
- **Actor shard count** — More shards = higher parallelism (diminishing returns after ~32)

---

### 18.3 Monitoring & Observability

**Structured Logging:**

Lobby emits structured logs (via `tracing` crate) for every pipeline stage:

```
2026-03-21T12:34:56.789Z INFO  pipeline{execution_id=550e8400... chain_id=1 from_address=0xaf9ce1...}:
  elapsed_ms=123 nonce=42 "nonce reserved"
```

**Key metrics to track:**

| Metric | Description | Alert Threshold |
|---|---|---|
| Pipeline latency | Time from submission to confirmation | > 60s (P95) |
| Nonce reservation failures | Database or sync errors | > 1% error rate |
| Broadcast failures | RPC rejections or timeouts | > 5% error rate |
| Validator timeouts | Transactions not confirmed after 5 min | > 10% timeout rate |
| Stale nonce count | Reserved nonces > 2 minutes old | > 10 active |

**Recommended tools:**
- **Prometheus** — Metrics collection (future Lobby integration)
- **Grafana** — Dashboards and alerting
- **Datadog / New Relic** — APM and distributed tracing
- **Sentry** — Error tracking and aggregation

---

### 18.4 Database Maintenance

**PostgreSQL Tuning:**

Lobby's database schema uses:
- **Indexed queries** on `(chain_id, from_address, execution_id)`
- **Frequent writes** to `nonce_assignments` and `sign_requests` tables
- **Periodic scans** by Sweeper and Scanner bots

**Recommended settings:**
```sql
-- Connection pooling
max_connections = 100
shared_buffers = 256MB

-- Write optimization
synchronous_commit = off  -- Accept risk of 1-2 second data loss on crash
wal_buffers = 16MB

-- Index efficiency
random_page_cost = 1.1  -- SSD storage assumed
```

**Backup strategy:**
- **Continuous archiving** — PostgreSQL WAL archiving to S3/GCS
- **Point-in-time recovery** — Daily full backups + WAL replay
- **Retention policy** — 7 days for transaction records, 30 days for audit logs

---

### 18.5 High Availability

**Redundancy:**

Lobby currently runs as a single-instance service. For production HA:

1. **Database replication** — PostgreSQL streaming replication (primary + standby)
2. **Load balancer** — Distribute traffic across multiple Lobby instances
3. **Shared state** — Redis for StatusRegistry (already implemented)
4. **Stateless actors** — Nonce/Sign/Broadcast actors can run on any instance

**Deployment architecture (future):**

```
                    ┌──────────────┐
                    │ Load Balancer│
                    └───────┬──────┘
                            │
        ┌───────────────────┼───────────────────┐
        ▼                   ▼                   ▼
    ┌────────┐          ┌────────┐          ┌────────┐
    │ Lobby  │          │ Lobby  │          │ Lobby  │
    │Instance│          │Instance│          │Instance│
    │   1    │          │   2    │          │   3    │
    └────┬───┘          └────┬───┘          └────┬───┘
         │                   │                   │
         └───────────────────┼───────────────────┘
                             ▼
                    ┌─────────────────┐
                    │  PostgreSQL HA  │
                    │ (Primary+Standby)│
                    └─────────────────┘
                             ▼
                    ┌─────────────────┐
                    │  Redis Cluster  │
                    │ (StatusRegistry)│
                    └─────────────────┘
```
---

## 19. API Overview

Lobby exposes two HTTP endpoints. Both require `Authorization: Bearer <api_token>` on every request.

### 19.1 POST /v1/transactions

Submits a transaction for processing. Returns immediately with an `execution_id`.

Lobby expects a JSON-RPC 2.0 envelope with `method: "eth_sendTransaction"` and one `params` entry in EIP-1193 format:

```bash
curl -X POST http://localhost:3000/v1/transactions \
  -H "Authorization: Bearer lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:<from_address>" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method":  "eth_sendTransaction",
    "params": [{
      "from":                 "<from_address>",
      "to":                   "<to_address>",
      "value":                "0x2386f26fc10000",
      "chainId":              "0x88bb0",
      "gas":                  "0x5208",
      "maxFeePerGas":         "0xba43b7400",
      "maxPriorityFeePerGas": "0x77359400"
    }],
    "id": 1
  }'
```

All numeric fields are hex strings. `gas`, `maxFeePerGas`, `maxPriorityFeePerGas`, `value`, `data`, and `accessList` are optional (default to zero/empty).

On success, the response is:
```json
{
  "jsonrpc": "2.0",
  "result": {
    "execution_id": "<uuid>",
    "status": "accepted"
  },
  "id": 1
}
```

On validation failure, a JSON-RPC error response is returned (HTTP 400/403).

### 19.2 GET /status/:execution_id

Returns the current pipeline status for the given `execution_id`. Clients should poll this until `status` is `confirmed_on_chain` or `failed`.

The response is a flat JSON object with a `status` field (snake_case tag) plus any status-specific fields:

```json
{ "execution_id": "...", "status": "nonce_reserved" }
{ "execution_id": "...", "status": "broadcasted", "tx_hash": "0x...", "messege": "Waiting for validation on-chain" }
{ "execution_id": "...", "status": "confirmed_on_chain", "tx_hash": "0x..." }
{ "execution_id": "...", "status": "failed", "stage": "broadcast", "reason": "..." }
{ "execution_id": "...", "status": "validator_timed_out", "message": "validator timed out, wait for confirmation" }
```

Returns HTTP 404 if the `execution_id` is unknown or has expired from the registry. Returns HTTP 400 if the `execution_id` is not a valid UUID.

> For full API semantics, error codes, and integration examples, see the separate [Lobby_API_Doc](docs/Lobby_API_Doc.md)`.

---

## 20. Appendices

### 20.1 Glossary

| Term | Definition |
|---|---|
| **Execution ID** | UUID v4 identifier assigned to every transaction submission |
| **Nonce** | Sequential transaction counter per Ethereum account |
| **EIP-1559** | Ethereum Improvement Proposal introducing base fee + priority fee gas model |
| **RLP** | Recursive Length Prefix — Ethereum's binary serialization format |
| **Actor** | Isolated async task with message-passing interface (Tokio async/await) |
| **Shard** | Partition of key-space routed to specific actor instance |
| **Semaphore** | Concurrency control primitive (limits max parallel pipelines) |
| **RelayHost** | Pipeline stage that validates and persists transaction intents |
| **Cortex** | Central orchestrator that coordinates the five-stage pipeline |

---

### 20.2 Supported Chains

| Chain | Chain ID (dec) | Chain ID (hex) | Explorer |
|---|---|---|---|
| **Ethereum Mainnet** | 1 | `0x1` | [etherscan.io](https://etherscan.io) |
| **Hoodi Testnet** | 560048 | `0x88bb0` | [hoodi.etherscan.io](https://hoodi.etherscan.io/) |
| **Polygon** | 137 | `0x89` | [polygonscan.com](https://polygonscan.com) |
| **Arbitrum One** | 42161 | `0xa4b1` | [arbiscan.io](https://arbiscan.io) |

> **Note:** Sepolia testnet (chain ID 11155111) was originally supported but will be deprecated by end of 2026. Hoodi testnet is the recommended replacement for development.

**Adding new chains:**

Contact your Lobby operator to configure RPC endpoints for additional chains. The operator must set:

```bash
RPC_ENDPOINT_<chain_id>=https://<rpc_url>
```

---

### 20.3 Gas Parameter Guidelines

Lobby **does not** currently support automatic gas estimation. You must provide:
- `gas` — Gas limit for transaction execution
- `maxFeePerGas` — Maximum total fee per gas unit (EIP-1559)
- `maxPriorityFeePerGas` — Miner tip per gas unit (EIP-1559)

**Recommended approach:**

1. **Use RPC `eth_estimateGas`** to determine appropriate `gas` limit
2. **Query `eth_feeHistory`** or gas tracker APIs (Etherscan, Blocknative) for current base fee
3. **Set fees:**
   - `maxPriorityFeePerGas`: 1-3 gwei (normal priority) or 5-10 gwei (high priority)
   - `maxFeePerGas`: `(current_base_fee × 2) + maxPriorityFeePerGas`

**Example calculation:**

```python
# Current network state
current_base_fee = 30_000_000_000  # 30 gwei

# Conservative settings (fast inclusion)
max_priority_fee = 2_000_000_000   # 2 gwei tip
max_fee = (current_base_fee * 2) + max_priority_fee
# Result: 62 gwei max fee

# Submit to Lobby
client.submit_transaction(
    to="0x...",
    gas=21000,
    max_fee_per_gas=max_fee,
    max_priority_fee_per_gas=max_priority_fee
)
```

**Common gas limits:**

| Operation | Typical Gas Limit |
|---|---|
| Simple ETH transfer | 21,000 |
| ERC-20 transfer | 65,000 |
| Uniswap swap | 150,000 - 300,000 |
| NFT mint | 100,000 - 500,000 |
| Complex DeFi interaction | 500,000 - 1,000,000 |

---

### 20.4 Running Lobby Locally

**Quick Start:**

```bash
# 1. Clone repository
git clone https://github.com/romanticNomad/Lobby.git
cd lobby

# 2. Start PostgreSQL and Redis servers on Docker.
cd database
docker compose up -d
sqlx migrate run

# 3. Configure environment
cat > .env << EOF
DATABASE_URL=postgresql://postgres:password@localhost/lobby
SERVER_ADDR=0.0.0.0:3000
REDIS_URL=redis://localhost:6379
RUST_LOG=info

# API key (example)
LOBBY_API_KEY_1=lobby_live_dev123:550e8400-e29b-41d4-a716-446655440000:<test_account_from_address>

# RPC endpoints (get keys from Alchemy/Infura)
RPC_ENDPOINT_1=https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY
RPC_ENDPOINT_560048=https://eth-hoodi.g.alchemy.com/v2/YOUR_KEY
EOF

# 4. Create test keys
cat > test_keys.json << EOF
{
  "test_account": {
    "pvt_key": "<test_account_pvt_key>",
    "pub_key": "<test_account_pub_key>",
    "address": "test_account_from_address"
  }
}
EOF

# 5. Expose environmnet variables to terminal (Bash)
source .env

# 6. Start Lobby
cargo run --release

# Expected outcome
INFO     ｉ [info]: "relation \"_sqlx_migrations\" already exists, skipping"
INFO     ｉ [info]: api_keys loaded: 1
INFO     ｉ [info]: rpc_endpoints loaded: [ChainId(560048), ChainId(1)]
INFO     ｉ [info]: custody accounts loaded: 1
INFO     ｉ [info]: StatusRegistry loaded | artifacts_count: 0
INFO     ｉ [info]: cortex online
INFO     ｉ [info]: bots spawned: monitoring status
INFO     ｉ [info]: lobby listening at: | address: 0.0.0.0:3000

# 7. Test submission
curl -X POST http://localhost:3000/v1/transactions \
  -H "Authorization: Bearer lobby_live_dev123:550e8400-e29b-41d4-a716-446655440000:test_account_from_address" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_sendTransaction",
    "params": [{
      "from": "test_account_from_address",
      "to": "<to_address>",
      "value": "0x2386f26fc10000",
      "chainId": "0x88bb0",
      "gas": "0x5208",
      "maxFeePerGas": "0xba43b7400",
      "maxPriorityFeePerGas": "0x77359400"
    }],
    "id": 1
  }'
```
**Expected output:**
```json
{
  "jsonrpc": "2.0",
  "result": {
    "execution_id": "9d3f7b2a-4c8e-4a1b-9f6d-8e5c3b2a1d0f",
    "status": "accepted"
  },
  "id": 1
}
```
---
*Built with Rust, Tokio, Axum, PostgreSQL, and Redis.*  
*Designed for developers who need reliable, low-latency blockchain transaction infrastructure.*

> **End of Architecture Guide**
