We are building a low-latency, high-concurrency blockchain transaction signing service called **Lobby**. 

## Architecture Overview

Lobby uses an **actor-based concurrency model** where:

- **Actors** are long-lived Tokio tasks that exclusively own mutable, order-sensitive state
- Each actor processes messages **sequentially** from an async message queue (mpsc channel)
- Actors make **authoritative decisions** about their owned state and persist facts to PostgreSQL
- Actors communicate via **typed message passing** (oneshot channels for request/response)

## Primary Actors

Lobby has four core actors, each owning a specific domain:

1. **RelayHost** - Receives raw transactions from DApps/bots, normalizes them into Lobby's internal transaction format
2. **Nonce** - Manages blockchain nonce allocation under concurrent load, prevents nonce collisions, tracks nonce state (reserved/finalized/released)
3. **Sign** - Signs transactions with private keys (HSM integration in production)
4. **Broadcast** - Submits signed transactions to blockchain RPC nodes, handles retries and confirmation tracking

Each actor:
- Has a dedicated PostgreSQL schema (e.g., `nonce.nonce_assignments`, `broadcast.broadcast_requests`)
- Uses **revision-based state tracking** with composite primary keys `(execution_id, revision)` for full audit trails
- Implements **atomic, TOCTOU-safe queries** using `INSERT ... SELECT ... WHERE NOT EXISTS` patterns
- Provides **idempotency** through lease-based deduplication (e.g., 5-minute windows)

## Concurrency Solution: Pipeline + Sharding

To achieve high throughput while maintaining correctness, Lobby uses a **two-layer concurrency model**:

### Layer 1: Pipeline Per Transaction
- Each incoming transaction spawns a **lightweight pipeline task** (cheap Tokio task)
- A Pipeline pool is used to prevent memory overflow
- The pipeline orchestrates sequential calls across actors: `RelayHost → Nonce → Sign → Broadcast`
- Pipelines run **concurrently** - 1000 requests = 1000 pipeline tasks executing in parallel
- Each pipeline holds **cloned actor handles** (`ActorHandle` wraps `mpsc::Sender`, cheap to clone)

### Layer 2: Sharded Actors (nonce actor example given blow)
- Instead of **one** nonce actor (bottleneck), Lobby runs **N sharded nonce actors (keyd by from_addredd)**
- Requests are routed to actors based on `hash(from_address) % N`
- Different addresses → different actors → **true parallelism**
- Same address → same actor → **sequential ordering** (critical for nonce safety)

Example with 16 shards:
```
Address 0xAAA... → Nonce Actor #3
Address 0xBBB... → Nonce Actor #11
Address 0xCCC... → Nonce Actor #3  (same shard, sequential processing)
```

This achieves:
- **High concurrency** - N actors processing in parallel
- **Correctness** - Sequential processing per address prevents nonce races
- **Low latency** - Minimal actor queue depth per shard
- Similarly logic can be used for **broadcast (keyed by chain_id)** and **sign (keyed by execution_id)**
- **RelayHost** does not need sharding since it is the entry point to the pipeline

## Tech Stack
- **Runtime**: Tokio (async Rust)
- **Database**: PostgreSQL (Docker-hosted, sqlx for compile-time query checking)
- **Message Passing**: `tokio::sync::mpsc` for actor queues, `tokio::sync::oneshot` for responses
- **Type Safety**: SQLx macros (`sqlx::query!`) for compile-time SQL verification
- **Axum**: Http requests handling and providing EIP-1193 evm adapter.

## Current State
We have implemented:
- Nonce, Broadcast and Sign actors with atomic reserve/resolve operations
- PostgreSQL schema with revision tracking and partial unique indexes
- TOCTOU-safe query patterns using `INSERT ... SELECT` with `WHERE NOT EXISTS`
- Idempotency handling via lease-based deduplication

## Pending implimentations:
- RelayHost actor
- A Validator task at the end of the pipeline to validate the block inclusion of the transaction
- EIP-1559 Gas estimation library
- Setting up the RpcProviderRegistry Dashmap
- Setting up the pipeline for orchestration of all the actors
- Wiring down all the crates in the main function
- Unit and Integration tests before launching the prototype

Key constraints:
- Must maintain ACID properties for state transitions
- Must handle actor crashes gracefully (lease expiration as recovery mechanism)
- Must support idempotent retries (same execution_id can be called multiple times)
- Prefer compile-time safety (sqlx::query! over runtime queries)

---

This doc:
1. Defines what actors are in Lobby's context
2. Explains the pipeline + sharding concurrency model
3. Provides enough context for any technical person/LLM to understand the architecture
4. Mentions key constraints and design principles

---
