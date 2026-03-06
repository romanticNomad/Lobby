# Lobby Architecture Guide

**Version:** 0.1.0 (Prototype)  
**Last Updated:** Sat Feb 28 23:37:25 2026  
**Target Audience:** Contributors, maintainers, and LLMs working with the Lobby codebase

---

## Table of Contents

1. [Introduction](#introduction)
2. [System Overview](#system-overview)
3. [Core Principles](#core-principles)
4. [The Kernel Crate](#the-kernel-crate)
5. [Actor Model](#actor-model)
6. [The Five Actors](#the-five-actors)
7. [Pipeline Orchestration](#pipeline-orchestration)
8. [Concurrency Model](#concurrency-model)
9. [Database Architecture](#database-architecture)
10. [RPC Provider Registry](#rpc-provider-registry)
11. [Status Registry & Polling](#status-registry--polling)
12. [Idempotency & State Safety](#idempotency--state-safety)
13. [Error Handling & Retry Logic](#error-handling--retry-logic)
14. [API Surface](#api-surface)
15. [Observability](#observability)
16. [Security Model](#security-model)
17. [Deployment](#deployment)
18. [Future Scope](#future-scope)

---

## Introduction

**Lobby** is a high-throughput, custodial EVM transaction signing service designed for applications that need to submit thousands of blockchain transactions per second while maintaining strict correctness guarantees around nonce ordering, idempotency, and state consistency.

Lobby treats transaction submission as a **multi-stage pipeline** where each stage is owned by a dedicated **actor** — a long-lived asynchronous task that processes requests sequentially and maintains exclusive ownership of mutable state. Actors communicate via typed message passing (Tokio mpsc channels) and persist state transitions to PostgreSQL with full audit trails.

### Key Characteristics

- **Custodial:** Private keys are managed by Lobby (in-memory, encrypted)
- **High-throughput:** Designed to handle 1000+ concurrent transaction pipelines
- **Correctness-first:** ACID guarantees, TOCTOU-safe queries, lease-based recovery
- **Idempotent:** Duplicate requests (same `execution_id`) are safely deduplicated
- **Auditable:** Every state transition is recorded with revision numbers in PostgreSQL

### Use Cases

- High-frequency trading bots that need guaranteed nonce ordering
- DeFi protocols submitting batch transactions (e.g., liquidations, rebalancing)
- NFT minting services that handle bursts of concurrent requests
- Cross-chain bridge operators that need reliable transaction broadcast

---

## System Overview

### Request Flow (End-to-End)

```
Client (DApp/Bot)
  │
  │ JSON-RPC: eth_sendTransaction
  │ Auth: Bearer <api_key>
  │
  ▼
┌─────────────────────────────────────────────────────────────────┐
│ Axum HTTP Server                                                │
│  ├─ Auth Middleware (api_key → ClientConfig)                    │
│  ├─ submit_transaction handler                                  │
│  │   ├─ Validate JSON-RPC                                       │
│  │   ├─ Normalize to Eip1559Transaction                         │
│  │   ├─ Generate ExecutionId (UUID v4)                          │
│  │   └─ Call orchestrator.submit()                              │
│  └─ get_transaction_status handler (polling)                    │
└─────────────────────────────────────────────────────────────────┘
  │
  │ Returns immediately: { execution_id, status: "accepted" }
  │
  ▼
┌─────────────────────────────────────────────────────────────────┐
│ cortex                                                    │
│  ├─ Acquire semaphore permit (backpressure control)             │
│  ├─ Spawn pipeline task (detached, runs in background)          │
│  └─ Update StatusRegistry: "accepted"                           │
└─────────────────────────────────────────────────────────────────┘
  │
  │ Pipeline (async task, runs to completion)
  │
  ▼
┌─────────────────────────────────────────────────────────────────┐
│ Pipeline Stages (sequential, with retries)                     │
│                                                                 │
│  1. RelayHost   → Persist transaction intent                   │
│  2. Nonce       → Reserve next available nonce                 │
│  3. Sign        → Sign transaction with private key            │
│  4. Broadcast   → Submit to blockchain RPC node                │
│  5. Validator   → Poll for on-chain confirmation               │
│                                                                 │
│  On Success: Finalize nonce, status → "confirmed"              │
│  On Failure: Release nonce, status → "failed"                  │
└─────────────────────────────────────────────────────────────────┘
  │
  │ Client polls: GET /status/{execution_id}
  │
  ▼
┌─────────────────────────────────────────────────────────────────┐
│ StatusRegistry (DashMap)                                        │
│  { execution_id → PipelineStatus }                              │
│    - "accepted" → "nonce_reserved" → "signed" → "broadcast"    │
│      → "confirmed" OR "failed"                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Technology Stack

- **Runtime:** Tokio (async Rust)
- **HTTP Server:** Axum
- **Database:** PostgreSQL (Docker-hosted, production will use AWS RDS)
- **Blockchain RPC:** Alloy (Ethereum client library)
- **Message Passing:** `tokio::sync::mpsc` (actor commands), `tokio::sync::oneshot` (responses)
- **Concurrency:** `DashMap` (lock-free concurrent HashMap), `Semaphore` (backpressure)
- **Query Safety:** SQLx compile-time checked macros

---

## Core Principles

Lobby's architecture is built on four foundational principles:

### 1. Sequential State Ownership (Actor Model)

**Problem:** Concurrent writes to shared state (e.g., nonce counters) cause race conditions.

**Solution:** Each piece of mutable state is exclusively owned by a single actor. Actors process requests **sequentially** from an mpsc queue, eliminating data races by construction.

**Example:** The Nonce actor for address `0xAAA...` is the **sole authority** on which nonce to assign next. All requests for that address are serialized through one queue.

### 2. Revision-Based Audit Trails

**Problem:** Debugging distributed systems requires understanding the full history of state transitions.

**Solution:** Every state change is recorded as a new row with a `(execution_id, revision)` composite primary key. The current state is `MAX(revision)`. This gives complete auditability without complex event sourcing.

**Example:**
```
execution_id  | revision | state      | updated_at
--------------+----------+------------+-------------------
abc-123-...   | 1        | reserved   | 2026-02-18 10:00
abc-123-...   | 2        | finalized  | 2026-02-18 10:01
```

### 3. Lease-Based Idempotency

**Problem:** Network retries or duplicate submissions can cause the same transaction to be processed twice.

**Solution:** Every request is tagged with an `execution_id` (UUID). State records have a 5-minute lease (tracked via `updated_at`). Duplicate requests within the lease window are rejected by partial unique indexes. After expiration, the state is considered stale and can be cleaned up.

**Example:**
```sql
CREATE UNIQUE INDEX idx_nonce_no_concurrent
  ON nonce.nonce_assignments (execution_id)
  WHERE state = 'reserved' AND updated_at > now() - interval '5 minutes';
```

### 4. TOCTOU-Safe Atomic Queries

**Problem:** `SELECT` + `INSERT` in separate queries allows time-of-check / time-of-use races.

**Solution:** All state mutations use `INSERT ... SELECT ... WHERE NOT EXISTS` patterns that execute atomically in a single database round-trip.

**Example (nonce reservation):**
```sql
INSERT INTO nonce.nonce_assignments
    (execution_id, revision, chain_id, from_address, nonce, state)
SELECT
    $1,
    COALESCE(
        (SELECT MAX(revision)
        FROM nonce.nonce_assignments
        WHERE execution_id = $1),
        0
    ) + 1,
    $2,
    $3,
    COALESCE(
        (SELECT MAX(nonce)
        FROM nonce.nonce_assignments
        WHERE chain_id = $2
        AND from_address = $3
        AND state IN ('reserved', 'released')),
        -1
    ) + 1,
    'reserved'
    WHERE NOT EXISTS (
        SELECT 1
        FROM nonce.nonce_assignments
        WHERE execution_id = $1
            AND (
                state = 'finalized'
                OR (
                    state = 'reserved'
                    AND updated_at > now() - interval '5 minutes'
                )
            )
    )
RETURNING nonce, revision
```

---

## The Kernel Crate

The **kernel** crate (`lobby-types` or `lobby-traits` in external references) contains all fundamental, cross-cutting types and traits that actors depend on. It has **zero business logic** — only data structures, type aliases, and trait definitions.

### Core Types

**Identity & Blockchain:**
- `ExecutionId` — Newtype over `uuid::Uuid`, uniquely identifies one transaction submission
- `ChainId` — Type alias for blockchain network ID (1 = Ethereum mainnet, 137 = Polygon, etc.)
- `TxNonce` — Type alias for Ethereum transaction nonce (monotonically increasing counter per address)
- `Address` — Re-export of `alloy_primitives::Address` (20-byte Ethereum address)

**Transactions:**
- `Eip1559Transaction` — Normalized EIP-1559 transaction structure:
  ```rust
  pub struct Eip1559Transaction {
      pub chain_id: ChainId,
      pub nonce: TxNonce,
      pub max_priority_fee_per_gas: U256,
      pub max_fee_per_gas: U256,
      pub gas_limit: U256,
      pub to: Option<Address>,      // None = contract creation
      pub value: U256,
      pub data: Bytes,
      pub access_list: Vec<(Address, Vec<U256>)>,
  }
  ```
- `SignedTransaction` — RLP-encoded, signed transaction ready for broadcast
- `BroadcastOutcome` — Result of submitting to RPC (contains `tx_hash: B256`)

**Client Context:**
- `ClientConfig` — Links an API request to a customer:
  ```rust
  pub struct ClientConfig {
      pub client_id: uuid::Uuid,
      pub from_address: Address,
  }
  ```

### Actor Traits

These traits define the public interface of each actor. Actor handles implement these via message passing to their internal engines.

```rust
#[async_trait]
pub trait IntentRelay: Send + Sync {
    async fn send_transaction(
        &self,
        execution_id: ExecutionId,
        txn: Eip1559Transaction,
        client_config: ClientConfig,
    ) -> Result<(), RelayHostError>;
}

#[async_trait]
pub trait NonceManager: Send + Sync {
    async fn reserve(
        &self,
        chain_id: ChainId,
        from: Address,
        execution_id: ExecutionId,
    ) -> Result<TxNonce, LocalError>;

    async fn resolve(&self, id: ExecutionId, outcome: bool) -> Result<(), LocalError>;
}

#[async_trait]
pub trait Signer: Send + Sync {
    async fn sign(
        &self,
        chain_id: ChainId,
        from: Address,
        execution_id: ExecutionId,
        txn: Eip1559Transaction,
    ) -> Result<SignedTransaction, LocalError>;
}

#[async_trait]
pub trait Broadcaster: Send + Sync {
    async fn broadcast(
        &self,
        chain_id: ChainId,
        from: Address,
        execution_id: ExecutionId,
        txn: SignedTransaction,
    ) -> Result<BroadcastOutcome, BroadcastError>;
}

#[async_trait]
pub trait Validator: Send + Sync {
    async fn validate(
        &self,
        chain_id: ChainId,
        execution_id: ExecutionId,
        tx_hash: B256,
    ) -> Result<ValidationOutcome, ValidatorError>;
}
```

### Error Types

Actors have their own error enums:
- `RelayHostError` — Database failures, validation errors
- `LocalError` — Shared by Nonce and Sign (database-only errors, no network I/O)
- `BroadcastError` — RPC failures, timeout errors
- `ValidatorError` — RPC failures, timeout, chain reorg detection

The orchestrator wraps these in `OrchestratorError` for unified error handling.

---

## Actor Model

### What is an Actor in Lobby?

An **actor** is a long-lived Tokio task that:
1. Owns a piece of **mutable state** (either in-memory or in PostgreSQL)
2. Processes **commands** sequentially from an `mpsc::Receiver<Command>`
3. Returns results via `oneshot::Sender<Result<T, E>>`
4. Persists state transitions to a **dedicated PostgreSQL schema**

Actors are the **sole authorities** on their owned state. No other code can read or write that state — all access goes through the actor's message queue.

### Anatomy of an Actor

Every actor follows the same pattern:

#### 1. Command Enum

Defines the messages the actor can receive:

```rust
pub enum NonceCommand {
    Reserve {
        chain_id: ChainId,
        from: Address,
        execution_id: ExecutionId,
        reply: oneshot::Sender<Result<TxNonce, LocalError>>,
    },
    Resolve {
        execution_id: ExecutionId,
        outcome: bool,  // true = finalize, false = release
        reply: oneshot::Sender<Result<(), LocalError>>,
    },
}
```

#### 2. Engine

The engine is the actor's event loop. It owns the database connection and processes commands until the channel closes:

```rust
pub struct NonceEngine {
    db: PgPool,
    rx: mpsc::Receiver<NonceCommand>,
}

impl NonceEngine {
    pub async fn run(mut self) {
        while let Some(cmd) = self.rx.recv().await {
            match cmd {
                NonceCommand::Reserve { chain_id, from, execution_id, reply } => {
                    let result = self.handle_reserve(chain_id, from, execution_id).await;
                    let _ = reply.send(result);
                }
                NonceCommand::Resolve { execution_id, outcome, reply } => {
                    let result = self.handle_resolve(execution_id, outcome).await;
                    let _ = reply.send(result);
                }
            }
        }
    }

    async fn handle_reserve(&self, ...) -> Result<TxNonce, LocalError> {
        // TOCTOU-safe INSERT ... SELECT query
    }

    async fn handle_resolve(&self, ...) -> Result<(), LocalError> {
        // Update state to 'finalized' or 'released'
    }
}
```

#### 3. Handle

The handle is a cheap-to-clone wrapper around `mpsc::Sender<Command>` that implements the actor's trait:

```rust
#[derive(Clone)]
pub struct NonceHandle {
    tx: mpsc::Sender<NonceCommand>,
}

#[async_trait]
impl NonceManager for NonceHandle {
    async fn reserve(&self, chain_id: ChainId, from: Address, execution_id: ExecutionId)
        -> Result<TxNonce, LocalError>
    {
        let (reply, rx) = oneshot::channel();
        self.tx.send(NonceCommand::Reserve { chain_id, from, execution_id, reply })
            .await
            .map_err(|_| LocalError::ActorStopped)?;
        rx.await.map_err(|_| LocalError::ActorStopped)?
    }

    async fn resolve(&self, execution_id: ExecutionId, outcome: bool)
        -> Result<(), LocalError>
    {
        let (reply, rx) = oneshot::channel();
        self.tx.send(NonceCommand::Resolve { execution_id, outcome, reply })
            .await
            .map_err(|_| LocalError::ActorStopped)?;
        rx.await.map_err(|_| LocalError::ActorStopped)?
    }
}
```

#### 4. Spawn Function

The public API to create an actor instance:

```rust
pub fn spawn_nonce_actor(db: PgPool, buffer_size: usize) -> NonceHandle {
    let (tx, rx) = mpsc::channel(buffer_size);
    let engine = NonceEngine::new(db, rx);

    tokio::spawn(async move {
        engine.run().await;
    });

    NonceHandle::new(tx)
}
```

### Why This Pattern?

**Sequential processing eliminates data races:**
- No locks, mutexes, or atomics needed
- State transitions are deterministic and auditable
- Failures are isolated (one actor panic doesn't corrupt other actors' state)

**Message passing decouples components:**
- Actors can be replaced, tested, or mocked independently
- The trait interface is stable (callers don't know about mpsc channels)
- Actors can be distributed across threads or even machines in the future

**Backpressure is automatic:**
- `mpsc::channel(buffer_size)` bounds the queue depth
- If an actor is overloaded, senders block until space is available
- This prevents unbounded memory growth under load

---

## The Five Actors

Lobby has five core actors, each owning a distinct phase of the transaction pipeline.

### 1. RelayHost

**Responsibility:** Accept and persist raw transaction intents from clients.

**State Owned:** `relay_host.relay_requests` table  
**Sharding:** Not sharded (single entry point)  
**Key Operations:**
- `send_transaction(execution_id, txn, client_config)` → Validates and records the request

**Why it exists:**
- Provides idempotency at the entry point (duplicate `execution_id` is rejected)
- Normalizes different client request formats into a canonical `Eip1559Transaction`
- Logs the original client parameters for audit trails

**Failure mode:**
Database error → Client receives error immediately, no pipeline is started.

---

### 2. Nonce

**Responsibility:** Manage monotonically increasing nonce counters per `(chain_id, from_address)`.

**State Owned:** `nonce.nonce_assignments` table  
**Sharding:** By `from_address` (same address → same actor → sequential nonce assignment)  
**Key Operations:**
- `reserve(chain_id, from, execution_id)` → Allocates next available nonce, marks it `'reserved'`
- `resolve(execution_id, outcome)` → Marks nonce as `'finalized'` (transaction confirmed) or `'released'` (transaction failed, nonce can be reused)

**Nonce Selection Logic:**
```
next_nonce = MAX(nonce WHERE state IN ('reserved', 'released')) + 1
```

This ensures that:
- Released nonces (from failed transactions) are recycled
- Multiple concurrent requests for the same address get sequential nonces (3, 4, 5, ...)
- No gaps in the nonce sequence

**Failure mode:**
- Reserve fails → No nonce assigned, pipeline exits, no cleanup needed
- Resolve fails → 5-minute lease expires, nonce becomes available again

---

### 3. Sign

**Responsibility:** Cryptographically sign transactions with the private key for `from_address`.

**State Owned:** `sign.sign_requests` table (audit trail only, no mutable state)  
**Sharding:** By `execution_id` (load balancing, signing is stateless)  
**Key Operations:**
- `sign(chain_id, from, execution_id, txn)` → Produces `SignedTransaction` (RLP-encoded bytes)

**Security Note:**
In the current prototype, private keys are loaded from environment variables and held in memory (unencrypted). Future versions will use AWS-backed envelope encryption: AWS KMS decrypts a master key, which is used in-memory to decrypt encrypted private key blobs.

**Failure mode:**
Database error or signing failure → Nonce is explicitly released via `nonce.resolve(execution_id, false)`.

---

### 4. Broadcast

**Responsibility:** Submit signed transactions to blockchain RPC nodes.

**State Owned:** `broadcast.broadcast_requests` table  
**RPC Owned:** Shared `RpcProviderRegistry` (DashMap of Alloy providers)  
**Sharding:** By `chain_id` (same chain → same actor → can maintain per-chain RPC state)  
**Key Operations:**
- `broadcast(chain_id, from, execution_id, signed_txn)` → Calls `eth_sendRawTransaction`, returns `tx_hash`

**Broadcast Strategy:**
1. Fetch provider from `RpcProviderRegistry` via `chain_id`
2. Call `provider.send_raw_transaction(rlp_bytes)`
3. Record `tx_hash` and state `'submitted'` in database
4. Return `BroadcastOutcome { tx_hash }`

**Failure mode:**
- RPC timeout or rejection → Retry up to N times (exponential backoff with full jitter)
- Hard failure after retries → Nonce is released, pipeline exits

**Why share `RpcProviderRegistry` with Validator?**
- Connection pooling: One HTTP client per chain, reused across actors
- Memory efficiency: ~10MB per provider × N chains (not × 2 actors)
- Consistent rate limiting: Both actors respect the same RPC provider limits

---

### 5. Validator

**Responsibility:** Poll blockchain RPC nodes to confirm transaction inclusion.

**State Owned:** `validator.validation_requests` table  
**RPC Owned:** Shared `RpcProviderRegistry` (same as Broadcast)  
**Sharding:** Not sharded (singleton actor, validation is I/O-bound and async)  
**Key Operations:**
- `validate(chain_id, execution_id, tx_hash)` → Polls `eth_getTransactionReceipt` until confirmed or timeout

**Validation Logic:**
1. Check cache: If this `execution_id` was validated in the last 5 minutes, return cached result
2. Poll loop (every 3 seconds):
   - Fetch `eth_getTransactionReceipt(tx_hash)`
   - If no receipt → transaction still pending, continue polling
   - If receipt with `status=0` → transaction reverted, return `NotIncluded`
   - If receipt with `status=1` → check confirmations (`current_block - tx_block`)
   - If confirmations ≥ required threshold → return `Included`
3. Timeout (default 5 minutes) → return `Timeout` error

**Confirmation Depth:**
- Default: 1 confirmation (fast, suitable for low-value transactions)
- Configurable: 3 confirmations (safer against shallow reorgs), 12 confirmations (Ethereum "finality")

**Failure modes:**
- `Included` → Nonce is finalized via `nonce.resolve(execution_id, true)`
- `NotIncluded` / `Timeout` / `Reverted` → Nonce is released via `nonce.resolve(execution_id, false)`

---

## Pipeline Orchestration

The **cortex** is the central coordinator that wires all five actors together into a sequential pipeline for each transaction submission.

### cortex Lifecycle

**Initialization (boot sequence in `main.rs`):**
1. Load configuration from environment (shard counts, buffer sizes, concurrency limits)
2. Spawn actor pools:
   - Nonce: N shards (keyed by `from_address`)
   - Sign: M shards (keyed by `execution_id`)
   - Broadcast: K shards (keyed by `chain_id`)
   - RelayHost: 1 instance (no sharding)
   - Validator: 1 instance (no sharding)
3. Create `RpcProviderRegistry` (shared by Broadcast and Validator)
4. Create `StatusRegistry` (DashMap for tracking pipeline progress)
5. Create `Semaphore` (bounds concurrent pipelines)
6. Return `OrchestratorHandle` (cheap `Arc`-backed clone)

**Runtime (per-request):**
1. HTTP handler calls `orchestrator.submit(execution_id, txn, client_config)`
2. cortex:
   - Acquires semaphore permit (blocks if at capacity, times out after 5 seconds → `BackpressureTimeout` error)
   - Registers `execution_id` in `StatusRegistry` as `"accepted"`
   - Spawns a detached Tokio task for the pipeline
   - Returns `Ok(())` immediately (handler responds to client with `{ execution_id, status: "accepted" }`)
3. Pipeline task runs to completion, updating `StatusRegistry` at each stage

### Pipeline Stages (Sequential Execution)

Each stage is wrapped in `retry_with_backoff(config, stage_name, operation)` which retries transient failures up to `max_attempts` (default: 2) with exponential backoff and full jitter.

**Stage 0: RelayHost (Optional Re-Invocation)**
- The HTTP handler already called `relay_host.send_transaction()` for immediate validation
- The pipeline *may* call it again for idempotency/retry coverage (depends on your RelayHost deduplication logic)
- If this call fails → No nonce reserved, pipeline exits immediately

**Stage 1: Nonce Reserve**
- Route to `nonce_pool.get(&ByAddress(from))` (shard by address)
- Call `nonce.reserve(chain_id, from, execution_id)` with retries
- On success → stamp `nonce` onto `Eip1559Transaction`, update status to `"nonce_reserved"`
- On failure → Exit pipeline (no cleanup needed, nonce was never reserved)

**Stage 2: Sign**
- Route to `sign_pool.get(&ByExecutionId(execution_id))` (shard by execution ID for load balancing)
- Call `sign.sign(chain_id, from, execution_id, txn)` with retries
- On success → update status to `"signed"`
- On failure → **Release nonce** via `nonce.resolve(execution_id, false)` with retries, then exit

**Stage 3: Broadcast**
- Route to `broadcast_pool.get(&ByChainId(chain_id))` (shard by chain)
- Call `broadcast.broadcast(chain_id, from, execution_id, signed_txn)` with retries
- On success → update status to `"broadcast"` with `tx_hash`, proceed to validation
- On failure → **Release nonce**, then exit

**Stage 4: Validator**
- Call `validator.validate(chain_id, execution_id, tx_hash)` with retries (singleton actor)
- On `Included` → **Finalize nonce** via `nonce.resolve(execution_id, true)`, update status to `"confirmed"`
- On `NotIncluded` / `Timeout` / `Reverted` → **Release nonce**, update status to `"failed"`

### Nonce Coordination (Critical for Correctness)

The nonce lifecycle has three states:
1. **Reserved** — Nonce is allocated, transaction is in-flight
2. **Finalized** — Transaction confirmed on-chain, nonce is consumed forever
3. **Released** — Transaction failed, nonce is available for reuse

**Finalize logic:**
```rust
async fn finalize_nonce(handle: &Arc<dyn NonceManager>, execution_id: ExecutionId, retry: &RetryConfig) {
    let result = retry_with_backoff(retry, "nonce_finalize", || {
        let h = Arc::clone(handle);
        async move { h.resolve(execution_id, true).await }
    }).await;

    if let Err(e) = result {
        // Non-fatal: state remains 'reserved', lease will expire after 5 minutes
        tracing::error!(%execution_id, %e, "nonce finalize failed — lease is fallback");
    }
}
```

**Release logic:**
```rust
async fn release_nonce(handle: &Arc<dyn NonceManager>, execution_id: ExecutionId, retry: &RetryConfig) {
    let result = retry_with_backoff(retry, "nonce_release", || {
        let h = Arc::clone(handle);
        async move { h.resolve(execution_id, false).await }
    }).await;

    if let Err(e) = result {
        // Non-fatal: lease will expire after 5 minutes
        tracing::error!(%execution_id, %e, "nonce release failed — lease expiry is fallback");
    }
}
```

**Why retries matter:**
A transient database error during `resolve()` would leave a dangling `'reserved'` nonce, blocking future transactions. Retries + 5-minute lease expiration provide defense-in-depth.

---

## Concurrency Model

Lobby achieves high throughput via a **two-layer concurrency model**.

### Layer 1: Pipeline-Level Parallelism

Each incoming request spawns a **lightweight pipeline task** (cheap Tokio task, ~2KB stack). These tasks run **concurrently** — 1000 requests = 1000 pipeline tasks executing in parallel.

**Backpressure control:**
A `Semaphore` with a configurable permit count (default: 17) bounds how many pipelines can run simultaneously. When the semaphore is exhausted, new requests block for up to 5 seconds before returning a `BackpressureTimeout` error (HTTP 429).

**Why limit concurrency?**
Unbounded parallelism would overwhelm:
- The database (too many concurrent connections)
- The RPC providers (rate limits)
- System memory (each pipeline holds transaction data in flight)

The semaphore ensures Lobby operates at **maximum sustainable throughput** without destabilizing.

### Layer 2: Actor-Level Sharding

Instead of one actor instance (which would be a bottleneck), Lobby runs **N sharded actors** for Nonce, Sign, and Broadcast. Requests are routed to shards via consistent hashing.

**Nonce Sharding (by `from_address`):**
```
hash(from_address) % N → shard_index
```
- **Same address → same shard → sequential nonce assignment** (correctness)
- **Different addresses → different shards → true parallelism** (throughput)

Example with 16 shards:
```
Address 0xAAA... → Nonce Actor #3
Address 0xBBB... → Nonce Actor #11
Address 0xCCC... → Nonce Actor #3  (same shard, serialized)
```

**Sign Sharding (by `execution_id`):**
```
hash(execution_id) % M → shard_index
```
- Signing is **stateless** (private key is fixed), so any shard can sign any transaction
- Sharding here is purely for **load balancing** (distribute CPU-intensive ECC operations)

**Broadcast Sharding (by `chain_id`):**
```
hash(chain_id) % K → shard_index
```
- **Same chain → same shard → can maintain per-chain RPC state** (connection pooling, nonce tracking at RPC level)
- **Different chains → different shards → parallel broadcasts**

**RelayHost & Validator:**
Not sharded. These are entry/exit points with minimal contention (RelayHost just writes to DB, Validator is I/O-bound).

### Routing Key Newtypes

To make shard routing explicit at call-sites, Lobby uses zero-cost wrapper types:

```rust
pub struct ByAddress<'a>(pub &'a Address);
pub struct ByExecutionId<'a>(pub &'a ExecutionId);
pub struct ByChainId<'a>(pub &'a ChainId);

// Usage
let nonce_handle = nonce_pool.get(&ByAddress(&from));
let sign_handle = sign_pool.get(&ByExecutionId(&execution_id));
let broadcast_handle = broadcast_pool.get(&ByChainId(&chain_id));
```

This prevents accidental routing errors (e.g., sharding Nonce by `chain_id` instead of `from_address`).

---

## Database Architecture

Lobby uses PostgreSQL as the **source of truth** for all actor state. Each actor owns a dedicated schema.

### Schema Isolation

- `relay_host.*` — Transaction intents
- `nonce.*` — Nonce assignments
- `sign.*` — Signing audit trail
- `broadcast.*` — Broadcast attempts and tx_hash records
- `validator.*` — Validation requests and outcomes

Actors are **completely isolated** at the database level:
- No foreign keys between schemas
- No cross-schema queries
- Each actor can be backed up, migrated, or scaled independently

### Revision-Based State Tracking

Every table uses a `(execution_id, revision)` composite primary key for full audit trails.

**Example: `nonce.nonce_assignments`**
```sql
CREATE TABLE nonce.nonce_assignments (
    execution_id      BYTEA NOT NULL,
    revision          BIGINT NOT NULL,
    chain_id          BIGINT NOT NULL,
    from_address      BYTEA NOT NULL,
    nonce             BIGINT NOT NULL,
    state             nonce.nonce_state NOT NULL, -- state in (reserved, finalized, released)
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (execution_id, revision)
);
```

**Reading current state:**
```sql
SELECT * FROM nonce.nonce_assignments
WHERE execution_id = $1
ORDER BY revision DESC
LIMIT 1;
```

**Writing new revision:**
```sql
INSERT INTO nonce.nonce_assignments (execution_id, revision, ...)
SELECT
    $1,                          -- execution_id
    MAX(revision) + 1,            -- next revision
    ...
FROM nonce.nonce_assignments
WHERE execution_id = $1;
```

### Lease-Based Idempotency

All actors use **5-minute leases** to prevent duplicate processing and enable automatic cleanup of stale state.

**Partial Unique Index (prevents concurrent processing):**
```sql
CREATE UNIQUE INDEX uniq_active_nonce
ON nonce.nonce_assignments (chain_id, from_address, nonce)
WHERE state IN ('reserved', 'finalized');
```

**Effect:**
- If two requests with the same `execution_id` arrive within 5 minutes, the second one is rejected by the unique index
- After 5 minutes, if the transaction never completed (e.g., server crash), the lease expires and the nonce becomes available for garbage collection

### TOCTOU-Safe Atomic Queries

All state mutations use **single-query atomicity** to avoid race conditions.

**Anti-pattern (TOCTOU vulnerability):**
```sql
-- Step 1: Check if nonce exists (TIME OF CHECK)
SELECT nonce FROM nonce.nonce_assignments WHERE execution_id = $1;

-- Step 2: Insert if not exists (TIME OF USE)
-- ❌ Another request could insert between these steps!
INSERT INTO nonce.nonce_assignments (...);
```

**Correct pattern (atomic):**
```sql
INSERT INTO nonce.nonce_assignments (execution_id, revision, nonce, state)
SELECT
    $1,
    COALESCE(MAX(revision), 0) + 1,
    COALESCE(MAX(nonce WHERE state IN ('reserved', 'released')), -1) + 1,
    'reserved'
WHERE NOT EXISTS (
    SELECT 1 FROM nonce.nonce_assignments
    WHERE execution_id = $1
      AND (state = 'finalized' OR (state = 'reserved' AND updated_at > now() - interval '5 minutes'))
)
RETURNING nonce, revision;
```

This executes as a **single database round-trip**, eliminating the possibility of interleaving with other requests.

### Auto-Update Triggers

All tables have `updated_at` triggers to track when state last changed:

```sql
-- Ensure updated_at is always called on state change
CREATE OR REPLACE FUNCTION nonce.touch_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.state IS DISTINCT FROM OLD.state THEN
        NEW.updated_at = now();
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_nonce_touch_updated_at
BEFORE UPDATE ON nonce.nonce_assignments
FOR EACH ROW
EXECUTE FUNCTION nonce.touch_updated_at();
```

This ensures `updated_at` is always accurate for lease expiration logic.

---

## RPC Provider Registry

The `RpcProviderRegistry` is a shared, lock-free concurrent HashMap that maps `ChainId` to Alloy `Provider` instances.

**Type Definition:**
```rust
pub type RpcProviderRegistry = Arc<DashMap<ChainId, Arc<dyn Provider + Send + Sync>>>;
```

**Why DashMap?**
- Lock-free reads (no global mutex contention)
- Sharded internally (64 shards by default), minimizing write contention
- `O(1)` lookups, same as `HashMap`

**Why Arc<dyn Provider>?**
- Providers are expensive to construct (~10MB, hold HTTP connection pools, retry state, rate limit counters)
- Sharing one instance across Broadcast and Validator saves TCP connections and memory

### Shared State Between Actors

Broadcast and Validator both read from the same `RpcProviderRegistry`:

```rust
// Broadcast actor
let provider = rpc_registry.get(&chain_id).ok_or(...)?;
provider.send_raw_transaction(rlp_bytes).await?;

// Validator actor
let provider = rpc_registry.get(&chain_id).ok_or(...)?;
provider.get_transaction_receipt(tx_hash).await?;
```

**Benefits:**
- **Connection pooling:** Alloy's internal HTTP client reuses connections across both actors
- **Memory efficiency:** One provider per chain (not one per actor per chain)
- **Consistent rate limiting:** Both actors respect the same RPC provider's rate limits

**Trade-off:**
If an RPC provider fails (e.g., rate limit exhausted), both Broadcast and Validator fail. This is acceptable because:
- RPC providers are critical infrastructure (pipeline is blocked anyway if they're down)
- Failure is **fail-fast** (both actors return errors immediately, nonce is released)
- You control which chains are supported (no dynamic chain addition that could poison the registry)

### Initialization

In the current prototype, the registry is built from environment variables at boot:

```bash
# Example (you may use a separate module in production)
export RPC_ENDPOINT_1=https://eth-mainnet.g.alchemy.com/v2/KEY
export RPC_ENDPOINT_137=https://polygon-mainnet.g.alchemy.com/v2/KEY
```

Future versions will use a dedicated initialization module with:
- Multiple fallback URLs per chain
- Health checks before adding to registry
- Dynamic provider rotation on failure

---

## Status Registry & Polling

The `StatusRegistry` is an in-memory, concurrent HashMap that tracks the current state of each pipeline.

**Type:**
```rust
pub struct StatusRegistry {
    inner: Arc<DashMap<ExecutionId, PipelineStatus>>,
}
```

**Pipeline States:**
```rust
pub enum PipelineStatus {
    Accepted,
    NonceReserved,
    Signed,
    Broadcast { tx_hash: String },
    Confirmed { tx_hash: String },
    Failed { stage: String, reason: String },
}
```

### Write Path (Pipeline → Registry)

The pipeline task updates the registry at each stage:

```rust
// After nonce reservation
status_registry.set(execution_id, PipelineStatus::NonceReserved);

// After signing
status_registry.set(execution_id, PipelineStatus::Signed);

// After broadcast
status_registry.set(execution_id, PipelineStatus::Broadcast {
    tx_hash: format!("{:#x}", tx_hash),
});

// After validation
status_registry.set(execution_id, PipelineStatus::Confirmed {
    tx_hash: format!("{:#x}", tx_hash),
});
```

### Read Path (Client → HTTP Endpoint)

Clients poll `GET /status/{execution_id}` to track progress:

**Request:**
```bash
curl http://localhost:3000/status/550e8400-e29b-41d4-a716-446655440000
```

**Response (in-progress):**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "broadcast",
  "tx_hash": "0xabc123..."
}
```

**Response (completed):**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "confirmed",
  "tx_hash": "0xabc123..."
}
```

**Response (failed):**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "failed",
  "stage": "validator",
  "reason": "transaction not included on-chain: timeout after 300s"
}
```

### Retention Policy

In the current prototype, entries are **never evicted** from the registry. For production:
- Option 1: Background task deletes entries older than N minutes
- Option 2: Migrate to PostgreSQL-backed status table (survives restarts)

Each entry is ~200 bytes, so 100,000 completed transactions ≈ 20MB — acceptable for a prototype.

---

## Idempotency & State Safety

Lobby guarantees that **duplicate requests are safely handled** via three mechanisms:

### 1. ExecutionId Deduplication

Every request is tagged with a globally unique `ExecutionId` (UUID v4, generated by the HTTP handler). All actors check for duplicate `execution_id` before processing:

```sql
WHERE NOT EXISTS (
    SELECT 1 FROM nonce.nonce_assignments
    WHERE execution_id = $1
      AND (state = 'finalized' OR (state = 'reserved' AND updated_at > now() - interval '5 minutes'))
)
```

**Effect:**
- If the same `execution_id` arrives twice within 5 minutes, the second request is rejected (no-op)
- After 5 minutes, the lease expires and the state is considered stale

### 2. Partial Unique Indexes

PostgreSQL enforces uniqueness on `execution_id` for active (non-expired) state:

```sql
CREATE UNIQUE INDEX idx_nonce_no_concurrent
  ON nonce.nonce_assignments (execution_id)
  WHERE state = 'reserved' AND updated_at > now() - interval '5 minutes';
```

This prevents race conditions at the database level — even if two requests pass the `NOT EXISTS` check simultaneously (impossible with the atomic `INSERT ... SELECT` pattern, but defense-in-depth), the unique index blocks the second one.

### 3. Revision-Based Audit Trails

Every state change creates a **new row** with an incremented `revision` number. This means:
- No data is ever overwritten (full history is preserved)
- You can replay the exact sequence of state transitions for debugging
- Rollback is possible (downgrade to a previous revision)

**Example: Nonce state progression**
```
execution_id                          | revision | state      | nonce | updated_at
--------------------------------------+----------+------------+-------+-------------------
550e8400-e29b-41d4-a716-446655440000  | 1        | reserved   | 42    | 2026-02-18 10:00
550e8400-e29b-41d4-a716-446655440000  | 2        | finalized  | 42    | 2026-02-18 10:01
```

### Safe Retry Behavior

**Scenario: Client retries after network timeout**

1. Client submits transaction → receives `{ execution_id: "abc-123", status: "accepted" }`
2. Network error before response arrives (client never sees it)
3. Client retries with **same request body**
4. Lobby generates a **new ExecutionId** (UUID v4) → treated as a fresh request

**Risk:** The original transaction may already be in-flight. Result: Two transactions, nonce collision.

**Mitigation (client-side):**
Clients must generate `execution_id` client-side (or use an idempotency key) if they need safe retries. Lobby will add support for client-provided `execution_id` in a future version.

---

## Error Handling & Retry Logic

Lobby uses **exponential backoff with full jitter** for all transient failures.

### Retry Strategy

**Configuration:**
```rust
pub struct RetryConfig {
    pub max_attempts: u32,        // Default: 2 (total tries = 3)
    pub base_delay: Duration,     // Default: 100ms
    pub max_delay: Duration,      // Default: 800ms
}
```

**Backoff Formula:**
```
window = min(max_delay, base_delay * 2^attempt)
delay  = rand(0, window)
```

**Why full jitter?**
Under load, many pipelines may fail simultaneously (e.g., RPC node hiccup, database connection spike). Full jitter scatters retries across the entire window, preventing a **thundering herd** re-spike.

**Example (2 retries, base=100ms, max=800ms):**
- Attempt 1 fails → sleep `rand(0, 100ms)`
- Attempt 2 fails → sleep `rand(0, 200ms)`
- Attempt 3 fails → give up

### Per-Stage Retry

Each pipeline stage wraps its actor call in `retry_with_backoff()`:

```rust
let nonce = retry_with_backoff(&config.retry, "nonce_reserve", || {
    let handle = Arc::clone(&nonce_handle);
    async move { handle.reserve(chain_id, from, execution_id).await }
})
.await?;
```

**Logged on each retry:**
```
WARN nonce_reserve: attempt failed, will retry after backoff
  stage: "nonce_reserve"
  attempt: 1
  delay_ms: 73
  error: "database connection timeout"
```

**Logged on final failure:**
```
ERROR nonce_reserve: all attempts exhausted, giving up
  stage: "nonce_reserve"
  attempt: 3
  error: "database connection timeout"
```

### Hard Failure Semantics

When a stage fails after all retries, the orchestrator:
1. Releases the nonce (if one was reserved)
2. Updates `StatusRegistry` to `Failed { stage, reason }`
3. Logs the error with structured fields (`execution_id`, `stage`, `error`)
4. Exits the pipeline (frees the semaphore permit)

**Failure stages:**
- **RelayHost** → No nonce reserved, exit immediately
- **Nonce Reserve** → No nonce reserved, exit immediately
- **Sign** → Release nonce, exit
- **Broadcast** → Release nonce, exit
- **Validator** → Release nonce, exit

---

## API Surface

Lobby exposes a minimal JSON-RPC API over HTTP.

### Endpoint: Submit Transaction

**Method:** `POST /v1/transactions`  
**Auth:** `Authorization: Bearer <api_key>`  
**Content-Type:** `application/json`

**Request Body (EIP-1193 format):**
```json
{
  "jsonrpc": "2.0",
  "method": "eth_sendTransaction",
  "params": [{
    "from": "0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb",
    "to": "0x5b35c4571b935076c853e9c3aab59b60d7b32daf",
    "value": "0xde0b6b3a7640000",
    "chainId": "0x1",
    "gas": "0x5208",
    "maxFeePerGas": "0x2540be400",
    "maxPriorityFeePerGas": "0x3b9aca00"
  }],
  "id": 1
}
```

**Optional Gas Fields:**
- `gas` (hex string) — Gas limit
- `maxFeePerGas` (hex string) — EIP-1559 max fee
- `maxPriorityFeePerGas` (hex string) — EIP-1559 priority fee

**Note:** In the current prototype, gas fields are **required**. Future versions will estimate gas automatically if omitted.

**Response (success):**
```json
{
  "jsonrpc": "2.0",
  "result": {
    "execution_id": "550e8400-e29b-41d4-a716-446655440000",
    "status": "accepted"
  },
  "id": 1
}
```

**Response (error):**
```json
{
  "jsonrpc": "2.0",
  "error": {
    "code": -32600,
    "message": "Invalid params: missing required field 'chainId'"
  },
  "id": 1
}
```

### Endpoint: Get Transaction Status

**Method:** `GET /status/{execution_id}`  
**Auth:** `Authorization: Bearer <api_key>`

**Response (in-progress):**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "broadcast",
  "tx_hash": "0xabc123..."
}
```

**Response (completed):**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "confirmed",
  "tx_hash": "0xabc123..."
}
```

**Response (failed):**
```json
{
  "execution_id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "failed",
  "stage": "validator",
  "reason": "timeout after 300s waiting for tx inclusion"
}
```

### Authentication

API keys follow the format:
```
LOBBY_API_KEY_<N>=<api_key>:<client_id>:<from_address>
```

**Example:**
```bash
LOBBY_API_KEY_1=lobby_live_abc123xyz:550e8400-e29b-41d4-a716-446655440000:0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb
```

**Fields:**
- `api_key` — Bearer token sent in `Authorization` header
- `client_id` — UUID v4 identifying the customer
- `from_address` — Ethereum address (checksummed or lowercase)

The auth middleware validates the API key and extracts `ClientConfig { client_id, from_address }` which is passed to the orchestrator.

---

## Observability

Lobby uses **structured logging** via the `tracing` crate for all operational visibility.

### Logging Strategy

**Log Levels:**
- `ERROR` — Unrecoverable failures (actor panics, hard failures after retries)
- `WARN` — Recoverable failures (retry attempts, lease expiration fallback)
- `INFO` — Major lifecycle events (actor spawned, pipeline completed, transaction confirmed)
- `DEBUG` — Detailed state transitions (nonce reserved, query executed)
- `TRACE` — (unused in production)

**Structured Fields:**
Every log event includes relevant context:
```rust
tracing::error!(
    %execution_id,
    stage = "nonce_reserve",
    %error,
    "pipeline hard-failed"
);
```

**Output:**
```
ERROR pipeline hard-failed
  execution_id: 550e8400-e29b-41d4-a716-446655440000
  stage: "nonce_reserve"
  error: "database connection timeout"
```

### Span-Based Tracing

The pipeline creates a span for each execution:

```rust
let span = tracing::info_span!(
    "pipeline",
    %execution_id,
    %chain_id,
    %from,
);

async move {
    tracing::info!("pipeline started");
    // ... stages ...
    tracing::info!("pipeline completed");
}
.instrument(span)
.await;
```

All logs within the pipeline task are automatically tagged with `execution_id`, `chain_id`, and `from`.

### Future: Metrics Collection

The current prototype has **no metrics collection**. Future versions will add:
- **Prometheus** or **Datadog** for quantitative metrics
- Key metrics:
  - Pipeline latency (p50, p95, p99)
  - Pipeline throughput (completions/sec)
  - Error rates by stage
  - Semaphore saturation (% of time at max concurrency)
  - Database query latency
  - RPC call latency and error rates

### Future: Trace IDs

To correlate logs across actors, future versions will propagate a **trace ID** (e.g., OpenTelemetry trace context) alongside `execution_id`.

---

## Security Model

The current prototype operates with **raw private keys in memory** for testing. Production security is planned but not yet implemented.

### Current (Prototype)

**Private Key Storage:**
- Loaded from environment variables: `PRIVATE_KEY_0xAAA=<hex_encoded_key>`
- Held in memory (unencrypted) by the Sign actor
- **NOT SUITABLE FOR PRODUCTION**

**API Authentication:**
- Bearer tokens in environment variables
- No encryption, no rotation
- **NOT SUITABLE FOR PRODUCTION**

### Future (Production)

**Envelope Encryption with AWS KMS:**
1. Private keys are encrypted with a data encryption key (DEK)
2. DEK is encrypted with AWS KMS (master key)
3. At boot:
   - Lobby calls AWS KMS to decrypt the DEK
   - DEK is held in memory (never written to disk)
   - Sign actor decrypts private keys on-demand using in-memory DEK
4. Private keys are held in memory (unencrypted) for low-latency signing
5. On shutdown, all keys are zeroed out

**Rationale:**
- Pre-runtime security check (Lobby won't start if KMS is unreachable)
- Low latency (no KMS call per signature, only at boot)
- Auditability (AWS CloudTrail logs all KMS calls)

**Additional Planned Security:**
- TLS for all RPC connections (currently HTTP in prototype)
- Encrypted database fields for sensitive data (e.g., private key blobs)
- API key rotation (currently static)
- Rate limiting per API key (currently none)

---

## Deployment

The current prototype is a **single binary** with PostgreSQL as the only external dependency.

### Containerization

**Lobby:**
```dockerfile
FROM rust:1.80 AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --package lobby

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/lobby /app/lobby
CMD ["/app/lobby"]
```

**PostgreSQL:**
```bash
docker run -d \
  --name lobby-postgres \
  -e POSTGRES_PASSWORD=secret \
  -e POSTGRES_DB=lobby \
  -p 5432:5432 \
  postgres:16
```

### Environment Variables

**Database:**
```bash
DATABASE_URL=postgres://postgres:secret@localhost:5432/lobby
```

**Server:**
```bash
SERVER_ADDR=0.0.0.0:3000
```

**cortex Config:**
```bash
NONCE_SHARDS=17
SIGN_SHARDS=17
BROADCAST_SHARDS=17
ACTOR_BUFFER_SIZE=64
PIPELINE_CONCURRENCY=17
PIPELINE_SEMAPHORE_TIMEOUT_MS=5000
```

**API Keys:**
```bash
LOBBY_API_KEY_1=lobby_live_abc:550e8400-e29b-41d4-a716-446655440000:0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb
```

**RPC Providers:**
```bash
RPC_ENDPOINT_1=https://eth-mainnet.g.alchemy.com/v2/KEY
RPC_ENDPOINT_560048="https://eth-hoodi.g.alchemy.com/v2/KEY"
```
### EVM Account Details (Lobby v0.1.0 -> prototype use only)

Lobby loads private keys from a `test_keys.json` file in the project root. Create this file with your test accounts:

```json
{
  "account1": {
    "pvt_key": "0xe74176dc8bcf2e5e6500a8f117a665ed44bcf448206c0cd23cb1228e61a2729c",
    "pub_key": "0x045ae5fcf5e6e9fafbec1155a363467bdea7eeb453bf1e68944b873f81871344dc38209d99d4af74dcbcba7da410353cbd45241cc9e91b863c170a8b1bda7e78a3",
    "address": "0xaf9ce11835e031df9c9db38a58fb75d8b70ffc92"
  },
  "account2": {
    "pvt_key": "0x25bf69fff0829179e3b4fcc1691c766b3126d78b8d80fe48f1322f074404ac5f",
    "pub_key": "0x040d382e58ebef374d9fe943891c11cbf0164ff9204370aaf1a6e8d8100773a8bfaf5342f2edee33263565c078c1babbf7ceed4c6dea9456dd72534e6803e105fa",
    "address": "0x5b35c4571b935076c853e9c3aab59b60d7b32daf"
  }
}
```

> **Security Warning:** The `test_keys.json` file contains **unencrypted private keys**. This is **only suitable for local testing with testnet accounts**. Never use this approach in production or with real funds.

### Production Deployment (Planned)

- **Cloud:** AWS (ECS/Fargate or EC2)
- **Database:** AWS RDS for PostgreSQL (Multi-AZ for high availability)
- **Secrets:** AWS Secrets Manager for API keys, KMS for private key encryption
- **Monitoring:** CloudWatch Logs + Prometheus + Grafana
- **Load Balancing:** Not yet planned (single instance for now)

---

## Future Scope

Lobby is a **working prototype**. The following features are planned but not yet implemented:

### 1. EIP-1559 Gas Estimation

**Current:** Clients must provide `gas`, `maxFeePerGas`, and `maxPriorityFeePerGas`.

**Future:**
- HTTP handler checks if gas fields are missing or zero
- If missing → fetch gas estimation from RPC (`eth_estimateGas`, `eth_feeHistory`)
- If broadcast fails due to underpricing → orchestrator bumps gas and retries
- Gas estimation module will use exponential fee bumping (e.g., +10% per retry)

**Implementation Plan:**
- Create `gas` module with `estimate_eip1559_fees(chain_id, txn)` function
- Integrate into submit handler (before calling orchestrator)
- Add retry logic in orchestrator for `BroadcastError::Underpriced`

---

### 2. Graceful Shutdown

**Current:** If an actor panics, the entire system fails. No graceful drain on shutdown.

**Future:**
- Implement actor supervision (restart crashed actors)
- On `SIGTERM`:
  - Stop accepting new requests
  - Drain in-flight pipelines (wait for completion, max 30s)
  - Close actor channels, flush database connections
  - Exit cleanly
- Add `/health` and `/readiness` endpoints for Kubernetes liveness/readiness probes

---

### 3. Shared Non-Actor State

**Current:** API keys and client configs are loaded from environment variables.

**Future:**
- Create `lobby.api_keys` and `lobby.client_configs` tables
- Add admin API for managing API keys (create, revoke, rotate)
- Add client-specific configuration (rate limits, gas price caps, allowed chains)

---

### 4. Metrics & Monitoring

**Current:** Structured logs only (no quantitative metrics).

**Future:**
- Prometheus metrics for:
  - Pipeline latency histograms (by stage)
  - Throughput (successful pipelines/sec)
  - Error rates (by stage, by error type)
  - Semaphore saturation (% of time at max concurrency)
  - Database connection pool usage
  - RPC call latency and error rates
- Grafana dashboards for real-time monitoring
- Alerting on error rate spikes, latency degradation, or semaphore saturation

---

### 5. Production Security

**Current:** Raw private keys in memory, no encryption.

**Future:**
- AWS KMS-backed envelope encryption (detailed in Security Model section)
- TLS for all RPC connections
- Encrypted database fields for sensitive data
- API key rotation
- Rate limiting per API key
- IP allowlisting for API access

---

### 6. Horizontal Scaling

**Current:** Single Lobby instance.

**Future:**
- Run multiple Lobby instances behind a load balancer
- Each instance has its own actor pools (sharded independently)
- Shared PostgreSQL (connection pooling via PgBouncer)
- **Challenge:** Status registry is in-memory — migrate to Redis or PostgreSQL for cross-instance visibility

---

### 7. Chain Reorg Handling

**Current:** Validator detects `NotIncluded` (transaction missing from chain) but doesn't distinguish between reorg and permanent failure.

**Future:**
- Detect chain reorgs by monitoring block hashes
- If transaction was in block N but block N is reorged out → re-broadcast automatically
- Add `max_reorg_retries` config

---

### 8. Gas Price Optimization

**Current:** Gas prices are fixed (provided by client or estimated once).

**Future:**
- Monitor pending transaction time in mempool
- If stuck for >30 seconds → bump gas price
- If confirmed quickly → learn from success, lower gas for future transactions
- Implement gas price prediction model (e.g., EIP-1559 base fee trends)

---

### 9. Multi-Signature Support

**Current:** Single private key per address.

**Future:**
- Support multi-sig wallets (e.g., Gnosis Safe)
- Coordinate off-chain signature collection across multiple signers
- Submit final transaction once threshold is met

---

## Appendix: Key Terms

- **Actor:** A long-lived Tokio task that exclusively owns mutable state and processes messages sequentially.
- **Command:** A message sent to an actor via mpsc channel (contains request data + oneshot reply channel).
- **Engine:** The actor's event loop implementation (owns database connection, processes commands).
- **Handle:** A cheap-to-clone wrapper around `mpsc::Sender<Command>` that implements the actor's trait.
- **Execution ID:** A globally unique identifier (UUID v4) for one transaction submission.
- **Lease:** A time-bound reservation of state (5 minutes in Lobby), enforced via `updated_at` timestamps.
- **Revision:** An incremental counter that tracks state transitions for a given `execution_id`.
- **TOCTOU:** Time-of-check / time-of-use — a race condition where state changes between checking and using it.
- **Shard:** One instance of a sharded actor (e.g., Nonce Actor #3 out of 17 total shards).
- **Pipeline:** The sequential execution of all five actor stages for one transaction submission.
- **Semaphore:** A concurrency primitive that limits the number of simultaneously running pipelines.
- **Status Registry:** An in-memory concurrent HashMap tracking the current state of each pipeline.
- **RPC Provider Registry:** A shared concurrent HashMap of Alloy providers (one per chain).

---

**End of Architecture Guide**

For API-specific documentation (request formats, error codes, rate limits), see `Client_API_Doc`.
