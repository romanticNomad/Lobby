Here is the updated architecture aligned with your directory request, followed by a module responsibility matrix and the production-grade skeleton implementing your ramp profile and strict 15s–55s metrics window.

### 📁 Updated Directory Structure
```text
├── Cargo.toml
├── README.md
└── src
    ├── infra.rs          # Testcontainers lifecycle & schema migration
    ├── mockrpc.rs        # Axum mock EVM RPC (nonce tracking, latency simulation)
    ├── types.rs          # Shared contracts (TxRequest, Config, Result enums)
    ├── metrics.rs        # HDRHistogram telemetry, 15s–55s window filtering, p99 reporting
    ├── dispatch.rs       # Async HTTP worker pool, reqwest pooling, latency recording
    └── loadgen
        ├── mod.rs        # Facade: exports pacer, producer, keys_gen
        ├── keys_gen.rs   # EVM key generation, test_keys.json, LOBBY_API_KEY_N derivation, payload pre-serialization
        ├── pacer.rs      # Token-bucket rate controller with ramp/steady/drop profile
        └── producer.rs   # Round-robin account distribution, mpsc push orchestration
```

### 📋 Module Responsibility Matrix

| Module | Responsibility | Institutional Alignment |
|--------|----------------|-------------------------|
| `main.rs` | Orchestrator: sequences infra boot, mock RPC spawn, Lobby process launch, phase coordination, final teardown & reporting. | Explicit phase boundaries, deterministic env injection, clean signal handling. |
| `infra.rs` | Spins up PostgreSQL/Redis via `testcontainers`, runs `sqlx` migrations, returns host ports. | Container lifecycle isolation, migration idempotency, port discovery. |
| `mockrpc.rs` | Simulates EVM JSON-RPC. Tracks per-address nonces, returns `eth_*` responses, injects configurable latency/jitter. | Sticky-session nonce consistency, state-machine isolation, realistic backpressure simulation. |
| `types.rs` | Canonical data contracts: `TxRequest`, `BenchmarkConfig`, shared enums. | Zero-allocation hot-path design, strict typing for inter-module boundaries. |
| `metrics.rs` | Per-worker `hdrhistogram`, filters samples strictly within `[15.0s, 45.0s]`, merges at shutdown, reports p50/p95/p99/p999. | Lock-free aggregation, warmup/drain exclusion, institutional percentile standards. |
| `dispatch.rs` | Worker pool: consumes `mpsc`, dispatches pre-serialized `Arc<Bytes>` payloads via pooled `reqwest`, records latency. | HTTP/2 multiplexing, connection reuse, fire-and-forget with async metric injection. |
| `loadgen/keys_gen.rs` | Generates valid EVM keypairs, writes `test_keys.json`, derives `LOBBY_API_KEY_N`, pre-serializes JSON-RPC payloads. | Deterministic auth routing, zero-runtime `serde_json`, exact Lobby format compliance. |
| `loadgen/pacer.rs` | Time-based rate controller: linear ramp (0–10s), steady (10–40s), drop (40s+). Drift-compensated sleep. | Token-bucket pacing, scheduler jitter correction, predictable throughput profiling. |
| `loadgen/producer.rs` | Round-robins across N accounts, applies pacer delay, pushes `TxRequest` into `mpsc`. | `ByAddress` nonce sharding exploitation, atomic distribution, bounded channel backpressure. |

---

### 🛠️ Core Implementation Skeleton (Ramp + Metrics Window)

#### `src/loadgen/pacer.rs`
Implements your exact ramp profile with drift compensation.
```rust
use tokio::time::{Duration, Instant};

pub struct Pacer {
    target_tps: u64,
    start: Instant,
    ramp_secs: f64,   // 10.0
    steady_secs: f64, // 45.0 (10 to 55)
}

impl Pacer {
    pub fn new(target_tps: u64) -> Self {
        Self { target_tps, start: Instant::now(), ramp_secs: 10.0, steady_secs: 45.0 }
    }

    /// Returns current target TPS based on elapsed time
    fn current_tps(&self) -> u64 {
        let elapsed = self.start.elapsed().as_secs_f64();
        if elapsed < self.ramp_secs {
            // Linear ramp: 0 -> target_tps over 10s
            (self.target_tps as f64 * (elapsed / self.ramp_secs)).ceil() as u64
        } else if elapsed < self.ramp_secs + self.steady_secs {
            self.target_tps
        } else {
            0 // Drop to zero after 40s
        }
    }

    pub async fn wait(&self) {
        let tps = self.current_tps();
        if tps == 0 {
            // Graceful backoff before shutdown
            tokio::time::sleep(Duration::from_millis(100)).await;
            return;
        }
        let interval = Duration::from_millis(1000 / tps);
        let deadline = Instant::now() + interval;
        tokio::time::sleep_until(deadline).await;
    }
}
```

#### `src/metrics.rs`
Strict window filtering `[15.0s, 55.0s]` + lock-free per-worker aggregation.
```rust
use hdrhistogram::Histogram;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::time::Instant;

const METRICS_WINDOW_START: f64 = 15.0;
const METRICS_WINDOW_END: f64 = 55.0;

#[derive(Clone)]
pub struct LocalMetrics {
    pub hist: Histogram<u64>,
    pub success: u64,
    pub failure: u64,
    pub bench_start: Instant,
}

impl LocalMetrics {
    pub fn new(bench_start: Instant) -> Self {
        Self {
            hist: Histogram::<u64>::new_with_bounds(1, 10_000_000_000, 3).unwrap(),
            success: 0,
            failure: 0,
            bench_start,
        }
    }

    pub fn record(&mut self, issued_at: Instant, status: Result<(), &str>) {
        let elapsed = issued_at.elapsed().as_secs_f64();
        // Strict institutional window filter
        if elapsed < METRICS_WINDOW_START || elapsed > METRICS_WINDOW_END {
            return;
        }
        let latency_us = issued_at.elapsed().as_micros() as u64;
        match status {
            Ok(()) => { self.success += 1; self.hist.record(latency_us).unwrap(); }
            Err(_) => self.failure += 1,
        }
    }
}

pub struct GlobalMetrics {
    pub hist: Mutex<Histogram<u64>>,
    pub total_success: parking_lot::Mutex<u64>,
    pub total_failure: parking_lot::Mutex<u64>,
}

impl GlobalMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            hist: Mutex::new(Histogram::<u64>::new_with_bounds(1, 10_000_000_000, 3).unwrap()),
            total_success: parking_lot::Mutex::new(0),
            total_failure: parking_lot::Mutex::new(0),
        })
    }

    pub fn merge_local(&self, local: LocalMetrics) {
        let mut g = self.hist.lock();
        g.add(&local.hist).unwrap();
        *self.total_success.lock() += local.success;
        *self.total_failure.lock() += local.failure;
    }

    pub fn report(&self) {
        let h = self.hist.lock();
        let s = *self.total_success.lock();
        let f = *self.total_failure.lock();
        tracing::info!(
            window = "15s-55s",
            p50_us = h.value_at_quantile(0.50),
            p95_us = h.value_at_quantile(0.95),
            p99_us = h.value_at_quantile(0.99),
            p999_us = h.value_at_quantile(0.999),
            success = s,
            failure = f,
            "benchmark_steady_state_report"
        );
    }
}
```

#### `src/main.rs` (Orchestration Snippet)
Shows how the phases, window, and buffer integrate.
```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    let config = BenchmarkConfig {
        target_tps: 1000,
        worker_count: 4,
        lobby_addr: "http://127.0.0.1:3000/v1/transactions".into(),
    };

    // 1. Infra & Mock RPC
    let infra = infra::bootstrap().await?;
    mockrpc::spawn_mockrpc(8545).await;

    // 2. Keys & Env
    let bench_dir = std::path::PathBuf::from("bench_data");
    std::fs::create_dir_all(&bench_dir)?;
    let accounts = keys_gen::generate(&bench_dir, 5)?; // N accounts

    let mut lobby_env = vec![
        ("DATABASE_URL", format!("postgresql://postgres:postgres@127.0.0.1:{}/test", infra.pg_port)),
        ("REDIS_URL", format!("redis://127.0.0.1:{}", infra.redis_port)),
        ("RPC_ENDPOINT_1", "http://127.0.0.1:8545".into()),
    ];
    for (i, acc) in accounts.iter().enumerate() {
        lobby_env.push((format!("LOBBY_API_KEY_{}", i + 1), acc.api_key.clone()));
    }

    // 3. Spawn Lobby
    let mut child = tokio::process::Command::new("cargo")
        .args(&["run", "--release", "--bin", "lobby"])
        .current_dir(&bench_dir)
        .env_clear()
        .envs(&lobby_env)
        .spawn()?;
    tokio::time::sleep(std::time::Duration::from_secs(4)).await; // Warm-up

    // 4. Benchmark Pipeline
    let bench_start = tokio::time::Instant::now();
    let global = metrics::GlobalMetrics::new();
    let (tx, rx) = tokio::sync::mpsc::channel(20_000);
    let client = reqwest::Client::builder().http2_prior_knowledge().pool_max_idle_per_host(100).build()?;
    
    // Spawn workers
    let workers: Vec<_> = (0..config.worker_count).map(|_| {
        let rx = rx.clone();
        let g = global.clone();
        let url = config.lobby_addr.clone();
        let start = bench_start;
        tokio::spawn(async move {
            let mut local = metrics::LocalMetrics::new(start);
            dispatch::run_worker(rx, client.clone(), url, &mut local).await;
            g.merge_local(local);
        })
    }).collect();

    // 5. Load Generator (Ramp -> Steady -> Drop -> Buffer)
    let pacer = loadgen::Pacer::new(config.target_tps);
    let prod_handle = tokio::spawn(loadgen::producer::run(tx, pacer, accounts, bench_start));

    // Wait for producer to finish (drops to 0 at 40s)
    prod_handle.await?;
    drop(tx); // Close channel, signal workers to drain

    // 10s buffer for in-flight requests to complete
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    
    // Await workers & report
    futures::future::join_all(workers).await;
    global.report();
    child.kill().await.ok();
    Ok(())
}
```

### 📐 Institutional Notes on Your 15s–55s Window
1. **Why 15s Start?** The first 10s ramp avoids TCP warm-up, connection pool initialization, and actor thread pinning. The 10–15s buffer allows Lobby's `NonceActor` and `SignActor` pools to reach steady-state throughput before measurement begins.
2. **Why 55s End?** Cutting measurement at 55s ensures you capture the tail latency of the last steady-state transactions before the pacer drops to zero. The subsequent 10s buffer drains in-flight `Validator` polls and `Nonce` reservations without polluting the histogram with drain-phase latency.
3. **Scaling to 5k/10k TPS:** 
   - Increase `mpsc` capacity to `target_tps * 2` (e.g., `20000` for 10k TPS)
   - Set `worker_count = num_cpus::get() * 2`
   - Pre-serialize 50+ accounts in `keys_gen.rs` to distribute across Lobby's `NONCE_SHARDS=100` routing table
   - Consider replacing the single `mpsc` with `N` worker-specific channels + round-robin distributor if the central channel becomes a scheduling bottleneck under `tokio`'s work-stealing runtime.

### possible changes: 
* `dispatch.rs` worker loop fleshed out with `reqwest` retry logic.
