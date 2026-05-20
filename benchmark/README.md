### `Cargo.toml`
```toml
[package]
name = "lobby-bench"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1.36", features = ["full"] }
reqwest = { version = "0.12", features = ["json", "http2"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
bytes = "1.5"
hdrhistogram = "7.5"
testcontainers = { version = "0.23", features = ["postgres", "redis"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "migrate"] }
axum = "0.8"
parking_lot = "0.12"
rand = "0.8"
thiserror = "1.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tokio-util = "0.7"
governor = "0.6"
```

### Updated Directory Structure
```text
├── Cargo.toml
├── README.md
└── src
    ├── infra.rs          # Testcontainers lifecycle & schema migration
    ├── main.rs           # Primary orchestrator of the bench harness
    ├── mockrpc.rs        # Axum mock EVM RPC (nonce tracking, latency simulation)
    ├── metrics.rs        # HDRHistogram telemetry, 5s–55s window filtering, p99 reporting
    └── loadgen
        ├── mod.rs        # Facade: exports trigger and keys module
        ├── keys.rs       # EVM key generation, test_keys.json, LOBBY_API_KEY_N derivation, payload pre-serialization
        └── trigger.rs    # builds lobby-tx request using random account distribution, and sequences tx for precise throughput
```

### Module Responsibility Matrix

| Module | Responsibility | Institutional Alignment |
|--------|----------------|-------------------------|
| `main.rs` | Orchestrator: sequences infra boot, mock RPC spawn, Lobby process launch, phase coordination, final teardown & reporting. | Explicit phase boundaries, deterministic env injection, clean signal handling. |
| `infra.rs` | Spins up PostgreSQL/Redis via `testcontainers`, runs `sqlx` migrations, returns host ports. | Container lifecycle isolation, migration idempotency, port discovery. |
| `mockrpc.rs` | Simulates EVM JSON-RPC. Tracks per-address nonces, returns `eth_*` responses, injects configurable latency/jitter. | Sticky-session nonce consistency, state-machine isolation, realistic backpressure simulation. |
| `metrics.rs` | Per-worker `hdrhistogram`, filters samples strictly within `[5.0s, 55.0s]`, merges at shutdown, reports p50/p95/p99/p999. | Lock-free aggregation, warmup/drain exclusion, institutional percentile standards. |
| `loadgen/keys.rs` | Generates valid EVM keypairs, writes `test_keys.json`, derives `LOBBY_API_KEY_N`. | Deterministic auth routing, zero-runtime `serde_json`, exact Lobby format compliance. |
| `loadgen/trigger.rs` | makes random selection across N accounts, returns built transaction matching the throughput set by the orchestrator, main.`    

### Implimentation Order of Modules

* infra
* loadgen
* mockrpc
* metrics -> alongside hdrhistogram integration to lobby pipeline
* main -> assembling the harness
