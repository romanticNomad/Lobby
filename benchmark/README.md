# Lobby Benchmark Architecture & Implementation Guide

## 1. Cargo.toml Configuration & `features` Tag

### Workspace Structure
```toml
# Cargo.toml (workspace root)
[workspace]
members = ["lobby", "lobby-bench"]
resolver = "2"

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
axum = "0.8.8"
reqwest = { version = "0.12", features = ["json", "http2"] }
testcontainers = "0.23"
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres"] }
metrics = "0.24"
hdrhistogram = "7"
alloy = { version = "1.6.0", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tracing = "0.1"
```

### `lobby-bench/Cargo.toml`
```toml
[package]
name = "lobby-bench"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "lobby-bench"
path = "src/main.rs"

[dependencies]
tokio.workspace = true
axum.workspace = true
reqwest.workspace = true
testcontainers.workspace = true
sqlx.workspace = true
metrics.workspace = true
hdrhistogram.workspace = true
alloy.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
rand = "0.8"
divan = "0.1" # Optional CLI harness
```

### The `features` Tag in Cargo
`features` enables conditional compilation via `#[cfg(feature = "...")]`. It gates code at compile-time, not runtime. In Lobby, we add `bench = []` to `lobby/Cargo.toml`:
```toml
# lobby/Cargo.toml
[features]
bench = []
```
**Usage:**
```rust
// pipeline.rs
#[cfg(feature = "bench")]
pub static BENCH_LATENCY: std::sync::LazyLock<Arc<Histogram>> = std::sync::LazyLock::new(|| {
    Arc::new(Histogram::new(3).unwrap()) // 3 sigfigs = 0.1% precision
});

#[cfg(feature = "bench")]
BENCH_LATENCY.record(latency.as_nanos() as u64).ok();
```
**Why:** Production builds (`cargo run --release`) omit histogram recording entirely (zero allocation, zero CPU overhead). Benchmark builds (`cargo run --release --features bench`) enable lock-free latency tracking. This is the 2026 standard for observability gating.

---

## 2. Module Algorithmic Breakdown

### `infra.rs` (DB/Redis Lifecycle + Postgres Tuning)
1. Spawn `testcontainers` for Postgres & Redis.
2. Wait for readiness: `pg_isready` / `redis-cli ping` (retry loop with 100ms backoff).
3. Apply tuned Postgres config via `container.with_env_var("POSTGRES_FLAGS", "...")`:
   ```
   -c shared_buffers=512MB -c max_connections=100 -c wal_level=minimal 
   -c fsync=off -c synchronous_commit=off -c checkpoint_timeout=300s
   ```
   *(Note: `fsync=off` is benchmark-only. Never use in prod.)*
4. Run `sqlx::migrate!().run(&pool).await`.
5. Implement `Drop` for container teardown on bench exit.

### `mockrpc.rs` (JSON-RPC Server + Nonce Tracking)
**Architecture:** `axum` router handling `POST /` with a stateful `MockRpcState` protected by `Arc<RwLock<InnerState>>`.

**Core Algorithm:**
```rust
struct InnerState {
    // Tracks next available nonce per address: {0xAddr: AtomicU64}
    nonce_ledger: DashMap<Address, AtomicU64>,
    // Simulates block confirmation delay
    confirmation_delay: Duration,
}

impl MockRpcState {
    async fn handle_request(&self, payload: JsonRpcRequest) -> JsonRpcResponse {
        match payload.method.as_str() {
            "eth_getTransactionCount" => {
                let addr = parse_from_addr(&payload.params)?;
                let nonce = self.nonce_ledger.entry(addr)
                    .or_insert_with(|| AtomicU64::new(0));
                JsonRpcResponse::success(payload.id, format!("0x{:x}", nonce.load(Ordering::Relaxed) + 5))
            }
            "eth_sendRawTransaction" => {
                let addr = extract_from_addr(&payload.params)?;
                let nonce = self.nonce_ledger.entry(addr)
                    .or_insert_with(|| AtomicU64::new(0));
                let current = nonce.fetch_add(1, Ordering::Relaxed);
                
                // Simulate network jitter & block production
                let jitter = rand::thread_rng().gen_range(1..5);
                tokio::time::sleep(Duration::from_millis(jitter)).await;

                // Return deterministic tx hash
                let mock_hash = format!("0x{:064x}", current ^ (addr.as_u64() as u64));
                JsonRpcResponse::success(payload.id, mock_hash)
            }
            "eth_getTransactionReceipt" => {
                // Always return success after confirmation_delay
                JsonRpcResponse::success(payload.id, serde_json::json!({
                    "status": "0x1",
                    "blockNumber": "0x1",
                    "transactionHash": payload.params[0]
                }))
            }
            _ => JsonRpcResponse::error(payload.id, -32601, "Method not found"),
        }
    }
}
```
**Institutional Features:**
- **Nonce Ledger:** `DashMap<Address, AtomicU64>` prevents TOCTOU and simulates mempool nonce progression without EVM execution.
- **Jitter Injection:** `tokio::time::sleep` + RNG prevents unrealistic 0ms latency spikes that hide pipeline backpressure.
- **Sticky Affinity Simulation:** If Lobby passes `chain_id` in headers, route to chain-specific `MockRpcState` instances to validate load-balancer routing.

### `loadgen.rs` (Account Generation + Ramp Phase)
**Account Gen:** Generate 250 `PrivateKeySigner` instances. Fund mock accounts via `eth_sendTransaction` RPC call pre-bench. Map 1:1 API keys.
**Ramp Algorithm:**
1. Define phases: `ramp(10s) → steady(30s) → cooldown(5s)`.
2. Use `tokio::time::interval` with 100ms ticks.
3. Calculate target requests per tick:
   ```rust
   let target_rps = if elapsed < 10.0 {
       (elapsed / 10.0) * 1000.0 // Linear ramp 0→1000
   } else if elapsed < 40.0 {
       1000.0
   } else {
       0.0
   };
   let per_tick = (target_rps / 10.0).ceil() as usize;
   ```
4. Gate concurrency with `tokio::sync::Semaphore::new(500)` to prevent TCP connection exhaustion.
5. Each tick spawns `per_tick` tasks via `JoinSet`, rotating through API keys round-robin.

### `main.rs` (Orchestrator)
State machine: `Infra → Spawn Lobby(—features bench) → Health Check → LoadGen → Metrics Drain → Teardown → Report`. Handles `SIGINT` via `tokio::signal::ctrl_c()`. Exports `benchmark_result.json`.

---

## 3. `hdrhistogram` Integration & Crate Comparison

### `metrics` vs `hdrhistogram` Crate
| Aspect | `metrics` Crate | `hdrhistogram` Crate |
|--------|----------------|----------------------|
| **Purpose** | Telemetry facade/macros (`counter!`, `histogram!`) | High-precision latency data structure |
| **Implementation** | Dynamic dispatch, global registry, allocates per update | Log-bucket compression, lock-free atomic updates, zero-allocation |
| **Percentiles** | Requires `metrics-util` aggregation; loses precision under skew | O(1) percentile extraction (`histogram.value_at_percentile(99.0)`) |
| **Overhead** | ~50-200ns per update + GC pressure | ~5-10ns per update, fixed memory footprint |

**Why `hdrhistogram` for Lobby:** `metrics` histograms fragment under 1000+ TPS due to lock contention and allocation. `hdrhistogram` uses a compressed log-scale bucket array updated via atomics. It retains precision across 6 orders of magnitude (1μs → 100s), critical for pipeline latency distribution.

### Integration Pattern
```rust
// Shared across pipeline actors
pub struct BenchMetrics {
    latency: Arc<HdrHistogram>,
}

impl BenchMetrics {
    pub fn record(&self, ns: u64) {
        self.latency.record(ns).ok(); // Drops if overflow; never panics
    }
    pub fn percentiles(&self) -> (f64, f64, f64) {
        (
            self.latency.value_at_percentile(50.0) as f64 / 1e6,
            self.latency.value_at_percentile(95.0) as f64 / 1e6,
            self.latency.value_at_percentile(99.0) as f64 / 1e6,
        )
    }
}
```
Inject `Arc<BenchMetrics>` into `Cortex` or pipeline actors. On bench exit, drain to JSON. No global registry, no dynamic dispatch.

---

## 4. Execution & 1000 TPS Validation

### Run Command
```bash
cargo run --release --bin lobby-bench -- --accounts 250 --target-tps 1000 --duration 40
```

### Success Criteria
Parse `benchmark_result.json` and validate:
```json
{
  "total_submitted": 40000,
  "total_broadcasted": 39850,
  "broadcast_tps": 1002.5,
  "latency_ms": { "p50": 12.4, "p95": 48.1, "p99": 112.7 },
  "error_rate": 0.003,
  "nonce_collisions": 0
}
```
**Thresholds for Institutional-Grade:**
1. `broadcast_tps >= 1000` sustained during steady-state window.
2. `p99_latency < 200ms` (pipeline processing only; mock RPC jitter excluded).
3. `error_rate < 0.1%` (only `NonceTooLow` or transient backpressure allowed).
4. `nonce_collisions == 0` (proves actor sharding + DB leases hold under load).

### Validation Logic
```rust
fn validate(result: &BenchmarkResult) -> Result<(), String> {
    if result.broadcast_tps < 995.0 {
        return Err(format!("TPS shortfall: {}", result.broadcast_tps));
    }
    if result.latency_ms.p99 > 200.0 {
        return Err(format!("p99 latency degraded: {}ms", result.latency_ms.p99));
    }
    if result.nonce_collisions > 0 {
        return Err("Nonce sequencing violated. Check actor sharding or DB SKIP LOCKED logic.".into());
    }
    Ok(())
}
```
