### `Cargo.toml`
```toml
[package]
name = "benchmark"
version = "0.1.0"
edition = "2024"
license = "Apache-2.0"

[dependencies]
tokio = { version = "1", features = ["full"] }
alloy = { version = "1.6.0", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
reqwest = { version = "0.13.3", features = ["json", "http2"] }
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "postgres",] }
k256 = { version = "0.13", features = ["ecdsa"] }
uuid = { version = "1.19", features = ["v4", "serde"] }
thiserror = "2.0.18"
sha3 = "0.10"
rand = "0.8"
hex = "0.4"
tracing = "0.1"
testcontainers = "0.26.0"
hdrhistogram = "7.5"
dashmap = "6.1.0"
axum = "0.8.8"
serde_json = "1.0.149"
anyhow = "1.0.102"
bytes = "1.11.1"
tokio-util = "0.7.18"
```

### Updated Directory Structure
```text
├── Cargo.toml
├── README.md
└── src
    ├── infra.rs             # Testcontainers lifecycle & schema migration
    ├── main.rs              # Primary orchestrator of the bench harness
    ├── metrics.rs         # HDRHistogram telemetry, 5s–55s window filtering, p99 reporting
    └── mockrpc.rs        # Axum mock EVM RPC (nonce tracking, latency simulation)
        ├── mod.rs          # Facade: exports router and state module elements
        ├── router.rs       # spawns mockrcp servers and provides handlers for eth requests
        └── state.rs         # manages the `ChainState` holding nonce_collection and receipt_collection for every rcp spawned.
    └── loadgen
        ├── mod.rs           # Facade: exports trigger and keys module elements
        ├── keys.rs          # EVM key generation, test_keys.json, LOBBY_API_KEY_N derivation, payload pre-serialization
        └── trigger.rs      # builds lobby-tx request using random account distribution, and sequences tx for precise throughput
```

### Module Responsibility Matrix

| Module               | Responsibility                                                                                                                                                                                              | Institutional Alignment                                                                                          |
|----------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------|
| `main.rs`            | Orchestrator: sequences infra boot, mock RPC spawn, Lobby process launch, phase coordination, final teardown & reporting.                                                                                   | Explicit phase boundaries, deterministic env injection, clean signal handling.                                   |
| `infra.rs`           | Spins up PostgreSQL/Redis via `testcontainers`, runs `sqlx` migrations, returns host ports.                                                                                                                 | Container lifecycle isolation, migration idempotency, port discovery.                                            |
| `mockrpc.rs`         | Simulates EVM JSON-RPC. Tracks per-address nonces, returns `eth_*` responses..                                                                                                                              | Sticky-session nonce consistency, state-machine isolation, realistic backpressure simulation.                    |
| `metrics.rs`         | Per-worker `hdrhistogram`, filters samples strictly within `[5.0s, 55.0s]`, merges at shutdown, reports p50/p95/p99/p999.                                                                                   | Lock-free aggregation, warmup/drain exclusion, institutional percentile standards.                               |
| `loadgen/keys.rs`    | Generates valid EVM keypairs, writes `test_keys.json`, derives `LOBBY_API_KEY_N`.                                                                                                                           | Deterministic auth routing, zero-runtime `serde_json`, exact Lobby format compliance.                            |
| `loadgen/trigger.rs` | Pre-serializing JSON-RPC payloads into Bytes. Makes random selection across N accounts, returns built transactions, matching the throughput set by the orchestrator, `main`, using `DynamicRateController`. | O(1) atomic cloning (zero-copy), simulates realistic traffic and leverages `Lobby`'s nonce sharding `ByAddress`. |

### Implimentation Order of Modules

* infra (done)
* loadgen (done)
* mockrpc (done)
* metrics → (done, alongside telemetry module in lobby::cortex)
* main → (done)
