# Lobby-Benchmark

> **Disclaimer**: This documentation was generated with the assistance of a Large Language Model (LLM) and has been thoroughly reviewed, and verified by the author to ensure technical accuracy.  
>
> **Last Update**: July 2, 2026  
> **Target Audience**: Quantitative Researchers, High-Frequency Trading (HFT) Engineers, EVM Infrastructure Architects, and DevOps/SRE teams.

---

## 1. Overview

`Lobby-Benchmark` is a deterministic load-generation and telemetry harness designed specifically to stress-test the `Lobby` EVM transaction service. Unlike ad-hoc load testing scripts that introduce uncontrolled variance (e.g., fluctuating public RPC latencies, network jitter, or database contention from shared environments), `Lobby-Benchmark` provides a hermetically sealed, fully deterministic testing environment.

It orchestrates a complete local infrastructure stack using `testcontainers`, spawns highly optimized mock EVM JSON-RPC servers using `Axum 0.8.8` and `Alloy 1.6.0`, and drives precise, mathematically bounded transaction throughput against the `Lobby` binary.

### Architectural Specialities

*   **Hermetic Infrastructure**: Spins up isolated PostgreSQL and Redis instances with deterministic port allocation.
*   **Stateful Mock RPC**: Simulates EVM state transitions, nonce-tracking, and receipt generation to simulate real-world network degradation.
*   **Mathematical Throughput Bounding**: Uses a `DynamicRateController` to enforce exact transactions-per-second (TPS) targets.
*   **Warmup/Drain Exclusion**: Implements strict `[5.0s, 35.0s]` time-window filtering on `HDRHistogram` samples to ensure telemetry reflects steady-state performance, ignoring JIT warmup, connection pooling initialization, and graceful drain phases.
*   **Zero-Copy Payload Serialization**: Pre-serializes JSON-RPC payloads into `bytes::Bytes` at boot, ensuring the load-generation phase operates with O(1) atomic cloning and zero runtime allocation overhead.

> **SECURITY WARNING**: The `loadgen` module generates EVM-compatible private keys and writes them to `test_keys.json` in plaintext. **Do not use these keys for storing real money or interacting with mainnet environments.** They are strictly for local benchmarking and testing purposes.

---

## 2. Table of Contents

1. [Executive Summary](#1-overview)
2. [Table of Contents](#2-table-of-contents)
3. [`main.rs` — The Primary Orchestrator](#3-mainrs--the-primary-orchestrator)
    - [3.1 Phase Coordination & Deterministic Environment Injection](#31-phase-coordination--deterministic-environment-injection)
    - [3.2 Signal Handling & Graceful Teardown](#32-signal-handling--graceful-teardown)
    - [3.3 The Benchmark Lifecycle Sequence](#33-the-benchmark-lifecycle-sequence)
4. [`infra.rs` — Container Lifecycle & Schema Migration](#4-infrars--container-lifecycle--schema-migration)
    - [4.1 Testcontainers Integration](#41-testcontainers-integration)
    - [4.2 SQLx Migration Idempotency](#42-sqlx-migration-idempotency)
    - [4.3 Dynamic Port Discovery & Network Isolation](#43-dynamic-port-discovery--network-isolation)
5. [`mockrpc.rs` — Axum Mock EVM RPC Server](#5-mockrpcrs--axum-mock-evm-rpc-server)
    - [5.1 `mod.rs` — Module Facade](#51-modrs--module-facade)
    - [5.2 `state.rs` — ChainState & State-Machine Isolation](#52-staters--chainstate--state-machine-isolation)
    - [5.3 `router.rs` — JSON-RPC Dispatch & Axum Handlers](#53-routerrs--json-rpc-dispatch--axum-handlers)
6. [`metrics.rs` — HDRHistogram Telemetry & Aggregation](#6-metricsrs--hdrhistogram-telemetry--aggregation)
    - [6.1 Lock-Free Per-Worker Aggregation](#61-lock-free-per-worker-aggregation)
    - [6.2 The `[5.0s, 35.0s]` Window Filtering Strategy](#62-the-50s-350s-window-filtering-strategy)
    - [6.3 Final Merge & Institutional Percentile Standards](#63-final-merge--institutional-percentile-standards)
7. [`loadgen` — Load Generation & Key Management](#7-loadgen--load-generation--key-management)
    - [7.1 `mod.rs` — Facade & Coordination](#71-modrs--facade--coordination)
    - [7.2 `keys.rs` — EVM Keypair Generation & API Key Derivation](#72-keysrs--evm-keypair-generation--api-key-derivation)
    - [7.3 `trigger.rs` — Transaction Sequencing & `DynamicRateController`](#73-triggerrs--transaction-sequencing--dynamicratecontroller)

---

## 3. `main.rs` — The Primary Orchestrator

The `main.rs` module serves as the central nervous system of the benchmark harness. `main.rs` is built entirely on the Tokio 1.x runtime, leveraging Rust 2024 edition features to guarantee explicit runtime-task boundaries and clean process teardowns.

### 3.1 Phase Coordination & Deterministic Environment Injection

The orchestrator operates on a strict, sequential phase model. No phase begins until the previous phase has emitted a verifiable health signal.

```rust
// Conceptual Phase Model in main.rs
pub enum BenchPhase {
    InfraBoot,
    MockRpcSpawn,
    LobbyProcessLaunch,
    LoadGeneration,
    TeardownAndReporting,
}
```

Environment variables for `actor-shards` and `pipeline-semaphore` limits are read from are read from a static `bench.env` file. For `ports ` and `keys`, `main.rs` dynamically constructs the environment matrix based on the ephemeral ports assigned by the `infra.rs` and `mockrpc.rs` modules. This ensures that parallel benchmark runs on the same CI/CD runner never experience port collisions.

### 3.2 Signal Handling & Graceful Teardown

Institutional infrastructure must handle `SIGINT` and `SIGTERM` gracefully to prevent database connection leaks and orphaned Docker containers. `main.rs` utilizes `tokio::signal::ctrl_c` combined with a `CancellationToken` (from `tokio-util 0.7.18`) to propagate shutdown signals across all spawned tasks.

When a termination signal is received:
1.  The `DynamicRateController` in `loadgen` ceases payload emission.
2.  In-flight HTTP requests in `reqwest` are allowed a 5-second drain window.
3.  The `Lobby` child process receives a `SIGTERM`.
4.  `testcontainers` invokes its `Drop` implementation, forcefully reaping the PostgreSQL and Redis Docker containers.

### 3.3 The Benchmark Lifecycle Sequence

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                       LOBBY-BENCHMARK LIFECYCLE                         │
├─────────────────────────────────────────────────────────────────────────┤
│  [ Phase 1: Infra Boot ]                                                │
│      ├── testcontainers::Postgres (Port 5432 -> Random)                 │
│      ├── testcontainers::Redis  (Port 6379 -> Random)                   │
│      └── sqlx migrate run (Idempotent Schema Bootstrap)                 │
│                                                                         │
│  [ Phase 2: Mock RPC Spawn ]                                            │
│      ├── Axum 0.8.8 Router Initialization                               │
│      ├── ChainState (DashMap) Allocation                                │
│      └── Bind to 127.0.0.1:0 (OS-assigned ephemeral port)               │
│                                                                         │
│  [ Phase 3: Lobby Process Launch ]                                      │
│      ├── Inject DATABASE_URL, REDIS_URL, RPC_ENDPOINT_MOCK              │
│      ├── Spawn `cargo run --release --bin lobby`                        │
│      └── Await Cortex Health Check (HTTP 200 on /health)                │
│                                                                         │
│  [ Phase 4: Load Generation ]                                           │
│      ├── Initialize HDRHistogram per-worker pools                       │
│      ├── DynamicRateController starts token bucket                      │
│      └── Fire-and-forget JSON-RPC payloads via reqwest                  │
│                                                                         │
│  [ Phase 5: Teardown & Reporting ]                                      │
│      ├── SIGTERM to Lobby PID                                           │
│      ├── Flush & Merge HDRHistograms                                    │
│      ├── Apply [5.0s, 55.0s] Time-Window Filter                         │
│      └── Emit p50/p95/p99/p999 Telemetry Matrix                         │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## 4. `infra.rs` — Container Lifecycle & Schema Migration

The `infra.rs` module abstracts the complexity of spinning up stateful dependencies. It relies heavily on `testcontainers 0.26.0` to provide Docker-based isolation, ensuring that the benchmark leaves zero footprint on the host machine's filesystem or network stack.

### 4.1 Testcontainers Integration

`infra.rs` defines custom image wrappers for PostgreSQL 18.3-alpine and Redis 8.6-alpine. It configures health checks natively within the container definitions, ensuring that the orchestrator only proceeds to the next phase when the databases are fully ready to accept connections and authenticate users.

### 4.2 SQLx Migration Idempotency

Lobby relies on `sqlx 0.8` for compile-time verified queries. The `infra.rs` module programmatically invokes the equivalent of `sqlx migrate run` against the ephemeral PostgreSQL instance. This guarantees that the schema is identical to the production environment, including all indices, triggers, and the critical `FOR UPDATE SKIP LOCKED` row-level locking mechanisms used by Lobby's nonce actor.

### 4.3 Dynamic Port Discovery & Network Isolation

Hardcoded ports (e.g., `localhost:5432`) are a primary source of flaky CI pipelines. `infra.rs` binds the container ports to `0.0.0.0:0`, allowing the host OS to assign random, available ephemeral ports. The module then extracts these mapped ports and exports them as `String` variables, which `main.rs` subsequently injects into the `Lobby` process environment.

---

## 5. `mockrpc.rs` — Axum Mock EVM RPC Server

Testing against live Alchemy or Infura endpoints introduces unacceptable variance. Network congestion, provider-side rate limiting, and mempool propagation delays make it impossible to isolate `Lobby`'s internal pipeline latency from external network latency. The `mockrpc` module solves this by providing a localized EVM JSON-RPC simulator.

### 5.1 `mod.rs` — Module Facade

The `mod.rs` file acts as a strict facade, exporting only the necessary router builders and state primitives to the orchestrator. It encapsulates the internal complexity of the Axum routing tree and the `ChainState` management, presenting a clean `spawn_mock_rpc()` API to `main.rs`.

### 5.2 `state.rs` — ChainState & State-Machine Isolation

The core of the mock RPC is the `ChainState` struct. It utilizes `dashmap 6.1.0` to provide O(1), lock-free concurrent access to simulated blockchain state.

```rust
pub struct ChainState {
    /// Address -> Expected Nonce (lock-free atomic advancement)
    pub nonce_collection: DashMap<Address, AtomicU64>,
    
    /// TxHash -> StaticReceipt map (appends on `BASE_RECEIPT`)
    pub receipt_collection: DashMap<TxHash, Arc<StaticReceipt>>,
}
```

This state-machine isolation ensures that when `Lobby` queries `eth_getTransactionCount` with the `"pending"` tag, it receives the exact nonce that the mock server expects for the next `eth_sendRawTransaction` call, perfectly simulating the sticky-session endpoint affinity that `Lobby`'s `Broadcast` actor relies upon.

### 5.3 `router.rs` — JSON-RPC Dispatch & Axum Handlers

Built on `Axum 0.8.8`, the router parses incoming JSON-RPC 2.0 payloads and dispatches them to specific handlers:

*   **`eth_sendRawTransaction`**: Decodes the RLP-encoded signed transaction using `alloy 1.6.0`. It verifies the ECDSA signature, checks the nonce against `nonce_collection`, and if valid, generates a simulated `TxHash` and inserts a pending receipt.
*   **`eth_getTransactionCount`**: Returns the current nonce for an address. Crucially, it supports the `"pending"` block tag, which is vital for testing Lobby's nonce-gap recovery mechanisms.
*   **`eth_getTransactionReceipt`**: Returns the simulated receipt. The mock server can be configured to delay the transition from `pending` to `confirmed` to test Lobby's `Validator` actor timeout logic.

---

## 6. `metrics.rs` — HDRHistogram Telemetry & Aggregation

Institutional telemetry must be mathematically rigorous. Averages (mean latency) are useless in high-frequency systems because they hide catastrophic tail-latency events. `metrics.rs` utilizes `hdrhistogram 7.5` to capture the full distribution of request latencies with high precision and low memory overhead.

### 6.1 Lock-Free Per-Worker Aggregation

To prevent lock contention on the telemetry pipeline, each load-generation worker thread maintains its own local `Histogram` instance. Recording a sample is a lock-free atomic operation. This ensures that the act of measuring latency does not perturb the latency itself (the observer effect).

### 6.2 The `[5.0s, 35.0s]` Window Filtering Strategy

One of the most critical features of `metrics.rs` is the strict time-window filtering. When a benchmark runs for 60 seconds, the data collected is not homogeneous:

1.  **`0.0s - 5.0s` (Warmup Phase)**: The Tokio runtime is spawning threads, `reqwest` is establishing TCP connection pools, and `Lobby` is warming up its internal `DashMap` and JIT-compiled closures. Latency here is artificially high.
2.  **`5.0s - 35.0s` (Steady State)**: The system is saturated and operating at peak throughput. This is the only data that matters for institutional SLA verification.
3.  **`35.0s - 45.0s` (Drain Phase)**: The orchestrator begins sending cancellation signals. In-flight requests may be dropped or delayed, causing artificial latency spikes.

`metrics.rs` tags every histogram sample with a timestamp. During the final merge phase, it aggressively discards any sample falling outside the `[5.0s, 35.0s]` window.

### 6.3 Final Merge & Institutional Percentile Standards

Once the load generation phase concludes, the module then extracts and reports the standard percentiles:

| Metric   | Description              | Institutional Target (Example) |
|:---------|:-------------------------|:-------------------------------|
| **p50**  | Median Latency           | < 15ms                         |
| **p95**  | 95th Percentile          | < 45ms                         |
| **p99**  | 99th Percentile          | < 120ms                        |
| **p999** | 99.9th Percentile (Tail) | < 350ms                        |

---

## 7. `loadgen` — Load Generation & Key Management

The `loadgen` module is responsible for generating the cryptographic material and network payloads required to drive the benchmark. It is engineered to eliminate runtime serialization overhead, ensuring that the bottleneck of the test is always `Lobby`'s pipeline, never the load generator.

### 7.1 `mod.rs` — Facade & Coordination

Exports the key generation utilities and the `DynamicRateController`. It ensures that the payload generation phase completes entirely before the `trigger` phase begins.

### 7.2 `keys.rs` — EVM Keypair Generation & API Key Derivation

Using the `k256` crate for secp256k1 elliptic curve operations, `keys.rs` generates `N` valid EVM keypairs. It formats these keys into the exact `test_keys.json` structure expected by Lobby's `JsonPolicyEngine`.

Furthermore, it derives the `LOBBY_API_KEY_N` environment variables. Lobby requires API keys in a highly specific format: `<token>:<client_id>:<from_address>`. `keys.rs` generates a UUIDv4 for the `client_id`, derives the `token` using the `lobby_live_` prefix, and binds it to the generated `from_address`. This guarantees that the Authorization stage (Stage 0) of Lobby's pipeline passes deterministically.

#### Zero-Copy Payload Pre-serialization

Standard load generators construct JSON payloads dynamically inside the request loop. This involves allocating `String` buffers and invoking `serde_json::to_vec` on every iteration, which causes severe GC/allocator pressure.

`keys.rs` pre-serializes the JSON-RPC payloads into `bytes::Bytes` at boot time. Because the nonces and gas parameters are mocked or managed externally by the `mockrpc`, the payload template remains static. During the load generation phase, the `trigger` module simply calls `.clone()` on the `Bytes` object. In Rust, cloning a `Bytes` object is an **O(1) atomic reference count increment**, resulting in zero memory allocation and zero serialization overhead during the critical test window.

### 7.3 `trigger.rs` — Transaction Sequencing & `DynamicRateController`

The `trigger.rs` module houses the `DynamicRateController`, a sophisticated token-bucket algorithm designed to enforce exact throughput (e.g., exactly 10,000 TPS).

#### Random Account Distribution

Lobby uses an actor-based sharding model where the `Nonce` actor is sharded `ByAddress`. If a load generator sends all traffic through a single address, it will bottleneck on a single Nonce actor shard, failing to test Lobby's concurrent pipeline architecture.

`trigger.rs` utilizes a pseudo-random distribution algorithm to route requests across the `N` generated accounts. This ensures that traffic is evenly distributed across Lobby's `NONCE_SHARDS`, fully saturating the actor pool and accurately reflecting Lobby's horizontal scaling capabilities.

#### Precise Throughput Enforcement
Naive load generators use `tokio::spawn` in a tight loop, saturating the local CPU and network buffers, which causes local backpressure that skews latency metrics. The `DynamicRateController` calculates the exact microsecond interval required between payload emissions to maintain the target TPS. It uses `tokio::time::sleep` to pace the requests and dynamically adjusts noise, and all observed latency is strictly a product of `Lobby`'s internal processing.

---
*Designed and engineered for the modern EVM stack. Built with Rust, Tokio, Axum, and Alloy.*
