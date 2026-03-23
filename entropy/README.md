# Lobby Testing & Benchmarking Strategy

**Version:** 0.1.0 (Prototype)  
**Last Updated:** March 22, 2026  
**Target Audience:** Contributors, QA engineers, and performance analysts

---

## Table of Contents

1. [Philosophy](#1-philosophy)
2. [Testing Architecture](#2-testing-architecture)
3. [Directory Structure](#3-directory-structure)
4. [Integration Testing](#4-integration-testing)
5. [Benchmark Harness](#5-benchmark-harness)
6. [monitor Stack](#6-monitor-stack)
7. [Running Tests](#7-running-tests)
8. [Interpreting Results](#8-interpreting-results)
9. [CI/CD Integration](#9-cicd-integration)
10. [Best Practices](#10-best-practices)
11. [Appendices](#11-appendices)

---

## 1. Philosophy

Entropy's testing strategy is built on three core principles:

### 1.1 Integration-First
Unit tests validate individual functions; integration tests validate **system behavior**. For a transaction pipeline with 5 actors, 2 databases, and external RPC dependencies, integration tests mirror production reality better than isolated unit tests.

### 1.2 Visual Proof of Performance
Marketing technical infrastructure requires visual evidence:
- **Live Grafana dashboards** during benchmark runs (throughput graphs, latency heatmaps)
- **Interactive HTML reports** with drill-down capability (per-stage latencies, error breakdowns)
- **Recorded terminal sessions** showing 1000+ TPS sustained throughput

---

## 2. Testing Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         ENTROPY STACK                           │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌──────────────┐      ┌──────────────┐      ┌──────────────┐   │
│  │   Load Gen   │─────▶│    Lobby     │─────▶│   Anvil      │   │
│  │  (Rust CLI)  │      │  (Pipeline)  │      │  (Devnets)   │   │
│  └──────┬───────┘      └──────┬───────┘      └──────────────┘   │
│         │                     │                                 │
│         │ metrics             │ metrics                         │
│         ▼                     ▼                                 │
│  ┌──────────────────────────────────────┐                       │
│  │         Prometheus Server            │                       │
│  │  (collects from Load Gen + Lobby)    │                       │
│  └──────────────┬───────────────────────┘                       │
│                 │                                               │ 
│                 ▼                                               │
│  ┌──────────────────────────────────────┐                       │
│  │         Grafana Dashboard            │                       │
│  │    (live visualization + alerts)     │                       │
│  └──────────────────────────────────────┘                       │ 
│                                                                 │
│  ┌──────────────────────────────────────┐                       │
│  │      PostgreSQL (Results DB)         │                       │
│  │   (historical benchmark storage)     │                       │
│  └──────────────────────────────────────┘                       │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 2.1 Component Roles

| Component | Purpose | Technology |
|-----------|---------|------------|
| **Load Generator** | Generate 1000+ TPS synthetic transactions | Rust CLI (Tokio + Reqwest) |
| **Lobby Instance** | The system under test | Production Lobby binary |
| **Anvil Devnets** | Local Ethereum nodes (no gas costs, instant blocks) | Foundry Anvil |
| **Prometheus** | Metrics scraping & storage (15s intervals) | Prometheus 2.x |
| **Grafana** | Real-time dashboards & alerting | Grafana 10.x |
| **PostgreSQL** | Benchmark result persistence | PostgreSQL 16.x |

### 2.2 Why This Stack?

**Rust Load Generator:**
- Native async/await (Tokio) → efficient connection pooling
- Zero GC pauses (consistent latency measurement)
- Easy integration with Lobby's error types for validation

**Anvil over Public Testnets:**
- **Deterministic block times** — Configure 1s blocks vs. testnet variability (12s ± jitter)
- **No rate limits** — Public RPC nodes throttle at 10-100 req/s
- **Instant account funding** — `anvil --accounts 1000 --balance 10000` vs. faucet queues
- **Reproducibility** — Same genesis state every run

**Prometheus over Custom Metrics:**
- Industry-standard pull model (Lobby exports `/metrics`, Prometheus scrapes)
- Rich query language (PromQL) for percentile calculations
- Native Grafana integration

---

## 3. Directory Structure

```
entropy/
├── Cargo.toml                          # Workspace root
├── README.md                           # This document
├── docker-compose.yml                  # Full stack (Anvil + Prometheus + Grafana + PostgreSQL)
│
├── crates/
│   ├── loadgen/                        # Load generator CLI
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs                 # CLI entry point
│   │   │   ├── generator.rs            # Transaction generation logic
│   │   │   ├── metrics.rs              # Prometheus exporter
│   │   │   └── config.rs               # Load profile config (TPS, duration, etc.)
│   │   └── profiles/                   # Predefined load profiles
│   │       ├── steady_1000tps.toml
│   │       ├── burst_5000tps.toml
│   │       └── ramp_up.toml
│   │
│   ├── integration-tests/              # Integration test suite
│   │   ├── Cargo.toml
│   │   ├── tests/
│   │   │   ├── happy_path.rs           # Full pipeline success
│   │   │   ├── nonce_conflict.rs       # Concurrent nonce requests
│   │   │   ├── rpc_failure.rs          # Broadcast retries
│   │   │   ├── validator_timeout.rs    # Scanner bot resolution
│   │   │   └── sweeper_bot.rs          # Stale nonce cleanup
│   │   └── fixtures/                   # Test data (sample transactions)
│   │       └── transactions.json
│   │
│   └── benchmark-harness/              # Benchmark orchestration
│       ├── Cargo.toml
│       ├── src/
│       │   ├── main.rs                 # Benchmark runner CLI
│       │   ├── orchestrator.rs         # Coordinate Load Gen + Lobby + Anvil
│       │   ├── collector.rs            # Gather metrics from Prometheus
│       │   └── reporter.rs             # Generate HTML/PDF reports
│       └── templates/
│           └── report.html.tera        # HTML report template
│
├── monitor/
│   ├── prometheus/
│   │   └── prometheus.yml              # Prometheus config (scrape targets)
│   ├── grafana/
│   │   ├── dashboards/
│   │   │   ├── lobby-overview.json     # Main dashboard (TPS, latency, errors)
│   │   │   └── actor-breakdown.json    # Per-actor metrics
│   │   └── provisions/
│   │       ├── datasources.yml         # Auto-configure Prometheus
│   │       └── dashboards.yml          # Auto-load dashboards
│   └── queries/
│       └── benchmark.sql               # Schema for storing benchmark runs
│
├── scripts/
│   ├── setup.sh                        # Install dependencies (Anvil, Prometheus, Grafana)
│   ├── benchmark_run.sh                # Wrapper script for benchmark execution
│   └── cleanup.sh                      # Stop containers, clean test data
│
└── results/                            # Generated benchmark outputs
    ├── 2026-03-22_1000tps_run1/
    │   ├── metrics.json                # Raw Prometheus query results
    │   ├── report.html                 # Interactive HTML report
    │   └── grafana_screenshot.png      # Dashboard snapshot
    └── historical.db                   # SQLite summary (quick comparisons)
```

---

## 4. Integration Testing

Integration tests validate **end-to-end behavior** of Lobby's pipeline under realistic conditions.

### 4.1 Test Categories

| Test Suite | Scenario | Success Criteria |
|------------|----------|------------------|
| **happy_path.rs** | Submit 100 txns, all confirm on-chain | 100% success rate, P95 < 5s |
| **nonce_conflict.rs** | 10 concurrent requests for same address | No duplicate nonces, all sequential |
| **rpc_failure.rs** | Kill Anvil node mid-broadcast, restart | Broadcast retries succeed after restart |
| **validator_timeout.rs** | Broadcast succeeds, block Anvil from mining | Validator times out, Scanner bot confirms later |
| **sweeper_bot.rs** | Reserve nonce, crash before broadcast | Sweeper releases nonce after 2min 5s |

### 4.2 Test Environment Setup

Tests use **testcontainers-rs** to spin up isolated PostgreSQL, Redis, and Anvil instances:

```rust
// integration-tests/tests/common/mod.rs
use testcontainers::{clients::Cli, images::postgres::Postgres, RunnableImage};
use std::process::{Command, Stdio};

pub struct TestEnvironment {
    pub postgres: Container<'static, Postgres>,
    pub redis: Container<'static, Redis>,
    pub anvil: AnvilInstance,
    pub lobby_addr: String,
}

impl TestEnvironment {
    pub async fn new() -> Self {
        let docker = Cli::default();
        
        // Start PostgreSQL
        let postgres = docker.run(RunnableImage::from(Postgres::default()));
        let pg_port = postgres.get_host_port_ipv4(5432);
        
        // Start Redis
        let redis = docker.run(RunnableImage::from(Redis::default()));
        let redis_port = redis.get_host_port_ipv4(6379);
        
        // Start Anvil (chain_id 31337, 1s block time)
        let anvil = AnvilInstance::spawn(AnvilConfig {
            block_time: Some(1),
            accounts: 100,
            balance: 10_000,
        }).await;
        
        // Start Lobby with test config
        let lobby_process = Command::new("cargo")
            .args(["run", "--bin", "lobby"])
            .env("DATABASE_URL", format!("postgres://postgres@localhost:{}", pg_port))
            .env("REDIS_URL", format!("redis://localhost:{}", redis_port))
            .env("RPC_ENDPOINT_31337", anvil.endpoint())
            .env("PIPELINE_CONCURRENCY", "10")
            .stdout(Stdio::piped())
            .spawn()
            .expect("Failed to start Lobby");
        
        // Wait for Lobby health check
        tokio::time::sleep(Duration::from_secs(2)).await;
        
        Self {
            postgres,
            redis,
            anvil,
            lobby_addr: "http://localhost:3000".to_string(),
        }
    }
}
```

### 4.3 Example Test: Nonce Conflict Resolution

```rust
// integration-tests/tests/nonce_conflict.rs
use entropy_integration_tests::common::TestEnvironment;
use lobby_client::LobbyClient;
use alloy::primitives::Address;
use std::sync::Arc;
use tokio::task::JoinSet;

#[tokio::test]
async fn concurrent_nonce_requests_are_sequential() {
    let env = TestEnvironment::new().await;
    let client = Arc::new(LobbyClient::new(&env.lobby_addr));
    let from_addr: Address = env.anvil.accounts()[0];
    
    // Submit 50 transactions concurrently from same address
    let mut tasks = JoinSet::new();
    for i in 0..50 {
        let client = Arc::clone(&client);
        tasks.spawn(async move {
            client.submit_transaction(SubmitRequest {
                from: from_addr,
                to: Address::random(),
                value: 1_000_000,
                chain_id: 31337,
                gas: 21_000,
                max_fee_per_gas: 1_000_000_000,
                max_priority_fee_per_gas: 1_000_000,
            }).await
        });
    }
    
    // Collect all execution_ids
    let mut execution_ids = Vec::new();
    while let Some(result) = tasks.join_next().await {
        let exec_id = result.unwrap().unwrap();
        execution_ids.push(exec_id);
    }
    
    // Poll until all confirmed
    let mut confirmed_nonces = Vec::new();
    for exec_id in execution_ids {
        let status = client.wait_for_confirmation(exec_id).await.unwrap();
        
        // Extract nonce from on-chain transaction
        let tx = env.anvil.get_transaction(status.tx_hash).await.unwrap();
        confirmed_nonces.push(tx.nonce);
    }
    
    // Assert nonces are sequential (0, 1, 2, ..., 49)
    confirmed_nonces.sort();
    assert_eq!(confirmed_nonces, (0..50).collect::<Vec<_>>());
}
```

### 4.4 CI/CD Integration Test Pipeline

```yaml
# .github/workflows/integration-tests.yml
name: Integration Tests

on:
  pull_request:
    branches: [main]
  push:
    branches: [main]

jobs:
  integration:
    runs-on: ubuntu-latest
    
    services:
      postgres:
        image: postgres:16
        env:
          POSTGRES_PASSWORD: test
        ports:
          - 5432:5432
      
      redis:
        image: redis:8-alpine
        ports:
          - 6379:6379
    
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      
      - name: Install Foundry (for Anvil)
        uses: foundry-rs/foundry-toolchain@v1
      
      - name: Cache Cargo
        uses: actions/cache@v3
        with:
          path: ~/.cargo
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
      
      - name: Run Integration Tests
        run: |
          cd entropy/crates/integration-tests
          cargo test --release -- --test-threads=1
        env:
          DATABASE_URL: postgres://postgres:test@localhost:5432/lobby_test
          REDIS_URL: redis://localhost:6379
          RUST_LOG: info
      
      - name: Upload Test Results
        if: failure()
        uses: actions/upload-artifact@v3
        with:
          name: test-logs
          path: entropy/crates/integration-tests/target/debug/test-*.log
```

---

## 5. Benchmark Harness

The benchmark harness orchestrates load generation, metrics collection, and report generation.

### 5.1 Load Generator Design

**Goal:** Generate 1000 TPS of realistic Ethereum transactions to Lobby's `/v1/transactions` endpoint.

**Architecture:**
```rust
// loadgen/src/generator.rs
use tokio::sync::Semaphore;
use reqwest::Client;
use std::sync::Arc;
use std::time::Instant;

pub struct LoadGenerator {
    lobby_url: String,
    http_client: Client,
    semaphore: Arc<Semaphore>,
    metrics: Arc<Metrics>,
}

impl LoadGenerator {
    pub async fn run(&self, profile: LoadProfile) -> Result<(), Error> {
        let start = Instant::now();
        let target_tps = profile.target_tps;
        let duration = profile.duration;
        
        // Calculate delay between requests to achieve target TPS
        let delay_micros = 1_000_000 / target_tps;
        let mut interval = tokio::time::interval(Duration::from_micros(delay_micros));
        
        let mut tasks = JoinSet::new();
        
        while start.elapsed() < duration {
            interval.tick().await;
            
            // Acquire semaphore permit (limits in-flight requests)
            let permit = self.semaphore.clone().acquire_owned().await.unwrap();
            
            let client = self.http_client.clone();
            let url = self.lobby_url.clone();
            let metrics = Arc::clone(&self.metrics);
            
            tasks.spawn(async move {
                let req_start = Instant::now();
                
                match submit_transaction(&client, &url).await {
                    Ok(exec_id) => {
                        let latency = req_start.elapsed();
                        metrics.record_success(latency);
                    }
                    Err(e) => {
                        metrics.record_error(&e);
                    }
                }
                
                drop(permit); // Release semaphore
            });
        }
        
        // Wait for all in-flight requests to complete
        while let Some(_) = tasks.join_next().await {}
        
        Ok(())
    }
}

async fn submit_transaction(client: &Client, url: &str) -> Result<ExecutionId, Error> {
    let tx = generate_random_transfer(); // Random to/value/gas
    
    let response = client
        .post(format!("{}/v1/transactions", url))
        .json(&tx)
        .send()
        .await?;
    
    let result: JsonRpcResponse = response.json().await?;
    Ok(result.result.execution_id)
}
```

### 5.2 Metrics Exporter

The load generator exposes Prometheus metrics at `:9091/metrics`:

```rust
// loadgen/src/metrics.rs
use prometheus::{Registry, Counter, Histogram, HistogramOpts};
use std::sync::Arc;

pub struct Metrics {
    registry: Registry,
    requests_total: Counter,
    requests_failed: Counter,
    latency_histogram: Histogram,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        let registry = Registry::new();
        
        let requests_total = Counter::new(
            "loadgen_requests_total",
            "Total requests sent"
        ).unwrap();
        
        let requests_failed = Counter::new(
            "loadgen_requests_failed_total",
            "Failed requests"
        ).unwrap();
        
        let latency_histogram = Histogram::with_opts(
            HistogramOpts::new(
                "loadgen_request_duration_seconds",
                "Request latency distribution"
            )
            .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0])
        ).unwrap();
        
        registry.register(Box::new(requests_total.clone())).unwrap();
        registry.register(Box::new(requests_failed.clone())).unwrap();
        registry.register(Box::new(latency_histogram.clone())).unwrap();
        
        Arc::new(Self {
            registry,
            requests_total,
            requests_failed,
            latency_histogram,
        })
    }
    
    pub fn record_success(&self, latency: Duration) {
        self.requests_total.inc();
        self.latency_histogram.observe(latency.as_secs_f64());
    }
    
    pub fn record_error(&self, _error: &Error) {
        self.requests_failed.inc();
    }
    
    pub fn export(&self) -> String {
        use prometheus::Encoder;
        let encoder = prometheus::TextEncoder::new();
        let metrics = self.registry.gather();
        
        let mut buffer = Vec::new();
        encoder.encode(&metrics, &mut buffer).unwrap();
        String::from_utf8(buffer).unwrap()
    }
}
```

### 5.3 Load Profiles

Predefined load profiles in TOML:

```toml
# loadgen/profiles/steady_1000tps.toml
name = "Steady 1000 TPS"
target_tps = 1000
duration_seconds = 60
max_in_flight = 5000
chain_id = 31337

[transaction_template]
from = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"  # Anvil account 0
gas = 21000
max_fee_per_gas = 1000000000
max_priority_fee_per_gas = 1000000
```

```toml
# loadgen/profiles/burst_5000tps.toml
name = "Burst 5000 TPS (10 seconds)"
target_tps = 5000
duration_seconds = 10
max_in_flight = 10000
chain_id = 31337
```

### 5.4 Benchmark Orchestrator

The harness coordinates all components:

```rust
// benchmark-harness/src/orchestrator.rs
use tokio::process::Command;
use std::time::Duration;

pub struct BenchmarkOrchestrator {
    lobby_binary: PathBuf,
    anvil_config: AnvilConfig,
    load_profile: LoadProfile,
}

impl BenchmarkOrchestrator {
    pub async fn run(&self) -> Result<BenchmarkResults, Error> {
        println!("🚀 Starting Anvil devnet...");
        let anvil = self.start_anvil().await?;
        
        println!("🏗️  Starting Lobby...");
        let lobby = self.start_lobby(&anvil).await?;
        
        println!("📊 Starting Prometheus...");
        let prometheus = self.start_prometheus().await?;
        
        println!("📈 Starting Grafana...");
        let grafana = self.start_grafana().await?;
        
        println!("⏳ Waiting 5s for warmup...");
        tokio::time::sleep(Duration::from_secs(5)).await;
        
        println!("🔥 Starting load generation ({} TPS for {}s)...",
                 self.load_profile.target_tps,
                 self.load_profile.duration_seconds);
        
        let start = Instant::now();
        self.run_load_generator().await?;
        let elapsed = start.elapsed();
        
        println!("✅ Load generation complete ({:.2}s)", elapsed.as_secs_f64());
        
        println!("📥 Collecting metrics from Prometheus...");
        let metrics = self.collect_metrics(&prometheus).await?;
        
        println!("📄 Generating report...");
        let report = self.generate_report(metrics).await?;
        
        println!("🧹 Cleaning up...");
        self.cleanup(anvil, lobby, prometheus, grafana).await?;
        
        Ok(report)
    }
    
    async fn collect_metrics(&self, prometheus: &PrometheusInstance) -> Result<Metrics, Error> {
        let client = reqwest::Client::new();
        
        // Query Prometheus for key metrics
        let queries = vec![
            ("lobby_pipeline_duration_seconds", "histogram_quantile(0.95, ...)"),
            ("lobby_requests_total", "sum(rate(lobby_requests_total[1m]))"),
            ("lobby_errors_total", "sum(rate(lobby_errors_total[1m]))"),
        ];
        
        let mut results = HashMap::new();
        for (name, query) in queries {
            let url = format!("{}/api/v1/query?query={}", prometheus.url, query);
            let response: PrometheusResponse = client.get(&url).send().await?.json().await?;
            results.insert(name.to_string(), response.data.result);
        }
        
        Ok(Metrics { results })
    }
}
```

---

## 6. monitor Stack

### 6.1 Prometheus Configuration

```yaml
# monitor/prometheus/prometheus.yml
global:
  scrape_interval: 15s
  evaluation_interval: 15s

scrape_configs:
  - job_name: 'lobby'
    static_configs:
      - targets: ['localhost:3000']  # Lobby exposes /metrics
  
  - job_name: 'loadgen'
    static_configs:
      - targets: ['localhost:9091']  # Load generator metrics
  
  - job_name: 'postgres'
    static_configs:
      - targets: ['localhost:9187']  # postgres_exporter
```

### 6.2 Lobby Metrics Endpoint

Lobby must expose Prometheus metrics. Add to `lobby/src/main.rs`:

```rust
use axum::routing::get;
use prometheus::{Encoder, TextEncoder, Registry};

async fn metrics_handler() -> String {
    let registry = GLOBAL_METRICS_REGISTRY.lock().unwrap();
    let encoder = TextEncoder::new();
    let metrics = registry.gather();
    
    let mut buffer = Vec::new();
    encoder.encode(&metrics, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}

// Add to router in main()
let app = Router::new()
    .route("/v1/transactions", post(submit_transaction))
    .route("/status/:execution_id", get(get_transaction_status))
    .route("/metrics", get(metrics_handler))  // ← Add this
    .with_state(state);
```

**Key Lobby Metrics to Expose:**

```rust
// Define in a new file: lobby/src/metrics.rs
use prometheus::{Registry, Histogram, Counter, HistogramOpts};
use lazy_static::lazy_static;
use std::sync::Mutex;

lazy_static! {
    pub static ref GLOBAL_METRICS_REGISTRY: Mutex<Registry> = {
        let registry = Registry::new();
        
        // Pipeline stage latencies
        let stage_duration = Histogram::with_opts(
            HistogramOpts::new(
                "lobby_pipeline_stage_duration_seconds",
                "Time spent in each pipeline stage"
            )
            .const_label("stage", "relayhost")
            .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0])
        ).unwrap();
        
        // Total requests
        let requests_total = Counter::new(
            "lobby_requests_total",
            "Total transaction submissions"
        ).unwrap();
        
        // Errors by stage
        let errors_total = Counter::new(
            "lobby_errors_total",
            "Errors by pipeline stage"
        ).unwrap();
        
        registry.register(Box::new(stage_duration)).unwrap();
        registry.register(Box::new(requests_total)).unwrap();
        registry.register(Box::new(errors_total)).unwrap();
        
        Mutex::new(registry)
    };
}
```

### 6.3 Grafana Dashboard

```json
{
  "dashboard": {
    "title": "Lobby Performance Overview",
    "panels": [
      {
        "title": "Throughput (TPS)",
        "type": "graph",
        "targets": [
          {
            "expr": "rate(lobby_requests_total[1m])",
            "legendFormat": "Lobby TPS"
          },
          {
            "expr": "rate(loadgen_requests_total[1m])",
            "legendFormat": "Load Gen TPS"
          }
        ]
      },
      {
        "title": "P95 Latency by Stage",
        "type": "graph",
        "targets": [
          {
            "expr": "histogram_quantile(0.95, rate(lobby_pipeline_stage_duration_seconds_bucket[1m]))",
            "legendFormat": "{{stage}}"
          }
        ]
      },
      {
        "title": "Error Rate",
        "type": "graph",
        "targets": [
          {
            "expr": "rate(lobby_errors_total[1m])",
            "legendFormat": "{{stage}}"
          }
        ]
      },
      {
        "title": "Active Pipelines (Semaphore Usage)",
        "type": "gauge",
        "targets": [
          {
            "expr": "lobby_pipeline_semaphore_active",
            "legendFormat": "In-flight pipelines"
          }
        ]
      }
    ]
  }
}
```

**Save as:** `monitor/grafana/dashboards/lobby-overview.json`

### 6.4 Docker Compose Stack

```yaml
# entropy/docker-compose.yml
version: '3.8'

services:
  anvil-mainnet-fork:
    image: ghcr.io/foundry-rs/foundry:latest
    command: >
      anvil
      --host 0.0.0.0
      --port 8545
      --chain-id 31337
      --block-time 1
      --accounts 1000
      --balance 10000
    ports:
      - "8545:8545"
  
  anvil-polygon-fork:
    image: ghcr.io/foundry-rs/foundry:latest
    command: >
      anvil
      --host 0.0.0.0
      --port 8546
      --chain-id 31338
      --block-time 2
      --accounts 1000
      --balance 10000
    ports:
      - "8546:8546"
  
  postgres:
    image: postgres:16
    environment:
      POSTGRES_USER: lobby
      POSTGRES_PASSWORD: benchmark_pass
      POSTGRES_DB: lobby_bench
    ports:
      - "5432:5432"
    volumes:
      - bench_pgdata:/var/lib/postgresql/data
  
  redis:
    image: redis:8-alpine
    ports:
      - "6379:6379"
  
  prometheus:
    image: prom/prometheus:v2.48.0
    ports:
      - "9090:9090"
    volumes:
      - ./monitor
    /prometheus/prometheus.yml:/etc/prometheus/prometheus.yml
      - bench_prometheus_data:/prometheus
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--storage.tsdb.path=/prometheus'
  
  grafana:
    image: grafana/grafana:10.2.0
    ports:
      - "3001:3000"
    volumes:
      - ./monitor
    /grafana/dashboards:/etc/grafana/provisions/dashboards
      - ./monitor
    /grafana/provisions:/etc/grafana/provisions
      - bench_grafana_data:/var/lib/grafana
    environment:
      GF_SECURITY_ADMIN_PASSWORD: admin
      GF_USERS_ALLOW_SIGN_UP: false

volumes:
  bench_pgdata:
  bench_prometheus_data:
  bench_grafana_data:
```

---

## 7. Running Tests

### 7.1 Prerequisites

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install Foundry (for Anvil)
curl -L https://foundry.paradigm.xyz | bash
foundryup

# Install Docker & Docker Compose
# (Platform-specific: https://docs.docker.com/get-docker/)
```

### 7.2 Quick Start: Integration Tests

```bash
cd entropy/crates/integration-tests

# Start dependencies (PostgreSQL, Redis, Anvil)
docker compose -f ../../docker-compose.yml up -d postgres redis anvil-mainnet-fork

# Wait 5 seconds for services to be ready
sleep 5

# Run all integration tests
cargo test --release

# Run specific test
cargo test --release nonce_conflict
```

**Expected Output:**
```
running 5 tests
test happy_path::full_pipeline_succeeds ... ok (4.23s)
test nonce_conflict::concurrent_requests_sequential ... ok (6.78s)
test rpc_failure::broadcast_retries_after_restart ... ok (8.12s)
test validator_timeout::scanner_bot_resolves ... ok (305.45s)
test sweeper_bot::releases_stale_nonces ... ok (125.89s)

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### 7.3 Quick Start: Benchmarks

```bash
cd entropy

# Start full monitor stack
docker compose up -d

# Wait for Grafana to be ready (takes ~10 seconds)
until curl -s http://localhost:3001/api/health | grep -q "ok"; do sleep 1; done

# Run benchmark
cd crates/benchmark-harness
cargo run --release -- \
  --profile ../../loadgen/profiles/steady_1000tps.toml \
  --output ../../results/$(date +%Y-%m-%d_%H-%M-%S)

# View results in Grafana
open http://localhost:3001/d/lobby-overview
# (Login: admin / admin)
```

### 7.4 Benchmark CLI Reference

```bash
# Run predefined profile
cargo run --release -- --profile profiles/steady_1000tps.toml

# Custom parameters
cargo run --release -- \
  --tps 2000 \
  --duration 120 \
  --chain-id 31337 \
  --output results/custom_run

# Multi-chain test
cargo run --release -- \
  --chains 31337,31338 \
  --tps-per-chain 500 \
  --duration 60

# Stress test (burst)
cargo run --release -- --profile profiles/burst_5000tps.toml
```

---

## 8. Interpreting Results

### 8.1 Key Metrics

| Metric | Target (1000 TPS) | Interpretation |
|--------|-------------------|----------------|
| **Sustained TPS** | ≥ 1000 | Actual throughput (measure via Prometheus `rate(lobby_requests_total[1m])`) |
| **P95 Latency (E2E)** | < 5 seconds | Time from submission to on-chain confirmation |
| **P95 Latency (Submission)** | < 100ms | Time from HTTP request to `accepted` response |
| **Error Rate** | < 0.1% | `lobby_errors_total / lobby_requests_total` |
| **Nonce Conflicts** | 0 | No duplicate nonces in on-chain transactions |
| **Validator Timeouts** | < 5% | Transactions timing out (due to nonce gaps) |

### 8.2 Latency Breakdown

Pipeline stages contribute to total latency:

```
Total Latency (P95 < 5s) = Submission + Nonce + Sign + Broadcast + Validator

Submission:   ~50ms   (HTTP parsing, auth, spawn task)
Nonce:        ~100ms  (DB query, atomic insert)
Sign:         ~50ms   (ECDSA signature generation)
Broadcast:    ~200ms  (RPC call to Anvil)
Validator:    ~2-4s   (polling for block inclusion, 1s block time)
```

**Optimization priorities:**
1. Validator is I/O-bound (dominated by block time) — reduce `poll_interval` or use websocket subscriptions
2. Nonce is CPU-bound at high concurrency — increase `NONCE_SHARDS`
3. Broadcast is network-bound — connection pooling (already present in Alloy)

### 8.3 Grafana Dashboard Interpretation

**Panel: Throughput (TPS)**
- **Load Gen TPS** should track target (e.g., flat line at 1000)
- **Lobby TPS** should match Load Gen (both ~1000)
- If Lobby TPS < Load Gen TPS → backpressure (semaphore exhausted)

**Panel: P95 Latency by Stage**
- Nonce spike → database contention (increase shards)
- Broadcast spike → RPC rate limiting (check Anvil logs)
- Validator spike → block time variance (expected on real chains)

**Panel: Error Rate**
- Zero errors = ideal
- Spikes during burst tests = retry exhaustion (tune `RetryConfig`)

**Panel: Active Pipelines**
- Should stay below `PIPELINE_CONCURRENCY` limit (default 17)
- At limit = backpressure active (HTTP 503s to clients)

### 8.4 HTML Report Structure

```html
<!-- Generated by benchmark-harness/src/reporter.rs -->
<!DOCTYPE html>
<html>
<head>
    <title>Benchmark Report: 2026-03-22 1000 TPS</title>
    <script src="https://cdn.plot.ly/plotly-2.27.0.min.js"></script>
</head>
<body>
    <h1>Lobby Benchmark Report</h1>
    <h2>Configuration</h2>
    <table>
        <tr><td>Target TPS</td><td>1000</td></tr>
        <tr><td>Duration</td><td>60 seconds</td></tr>
        <tr><td>Chain</td><td>Anvil (31337)</td></tr>
        <tr><td>Lobby Version</td><td>0.1.0</td></tr>
    </table>
    
    <h2>Results Summary</h2>
    <table>
        <tr><td>Achieved TPS</td><td>1,023</td><td>✅</td></tr>
        <tr><td>P95 Latency</td><td>4.2s</td><td>✅</td></tr>
        <tr><td>Error Rate</td><td>0.02%</td><td>✅</td></tr>
        <tr><td>Nonce Conflicts</td><td>0</td><td>✅</td></tr>
    </table>
    
    <h2>Latency Distribution</h2>
    <div id="latency-histogram"></div>
    <script>
        var data = [{
            x: [0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0],
            y: [12, 145, 789, 3456, 8901, 9234, 4321, 876, 123, 45],
            type: 'bar'
        }];
        Plotly.newPlot('latency-histogram', data);
    </script>
    
    <h2>Throughput Over Time</h2>
    <div id="throughput-timeseries"></div>
    <!-- ... -->
</body>
</html>
```

---

## 9. CI/CD Integration

### 9.1 GitHub Actions: Integration Tests

Already covered in section 4.4. Runs on every PR.

### 9.2 Nightly Regression Benchmarks

```yaml
# .github/workflows/nightly-benchmark.yml
name: Nightly Performance Regression

on:
  schedule:
    - cron: '0 2 * * *'  # 2 AM UTC daily
  workflow_dispatch:

jobs:
  benchmark:
    runs-on: ubuntu-latest
    
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      
      - name: Install Foundry
        uses: foundry-rs/foundry-toolchain@v1
      
      - name: Start Docker Stack
        run: |
          cd entropy
          docker compose up -d
          sleep 10  # Wait for services
      
      - name: Run Benchmark
        run: |
          cd entropy/crates/benchmark-harness
          cargo run --release -- \
            --profile ../../loadgen/profiles/steady_1000tps.toml \
            --output ../../results/nightly-$(date +%Y-%m-%d)
      
      - name: Store Results in PostgreSQL
        run: |
          cd entropy
          psql $DATABASE_URL -c "
            INSERT INTO benchmark_runs (date, tps, p95_latency, error_rate)
            VALUES (NOW(), 1023, 4.2, 0.0002);
          "
        env:
          DATABASE_URL: postgres://lobby:benchmark_pass@localhost:5432/lobby_bench
      
      - name: Compare vs. Baseline
        run: |
          cd entropy/scripts
          python compare_baseline.py \
            --current results/nightly-$(date +%Y-%m-%d)/metrics.json \
            --baseline results/baseline.json \
            --threshold 10  # Fail if >10% regression
      
      - name: Upload Report
        uses: actions/upload-artifact@v3
        with:
          name: benchmark-report
          path: entropy/results/nightly-*/report.html
      
      - name: Notify on Regression
        if: failure()
        uses: slackapi/slack-github-action@v1
        with:
          payload: |
            {
              "text": "⚠️ Lobby performance regression detected! Check GitHub Actions."
            }
        env:
          SLACK_WEBHOOK_URL: ${{ secrets.SLACK_WEBHOOK }}
```

### 9.3 Baseline Comparison Script

```python
# entropy/scripts/compare_baseline.py
import json
import sys

def compare(current_file, baseline_file, threshold_pct):
    with open(current_file) as f:
        current = json.load(f)
    with open(baseline_file) as f:
        baseline = json.load(f)
    
    metrics = ['tps', 'p95_latency', 'error_rate']
    
    for metric in metrics:
        current_val = current[metric]
        baseline_val = baseline[metric]
        
        # For latency/errors, lower is better
        if metric in ['p95_latency', 'error_rate']:
            regression_pct = ((current_val - baseline_val) / baseline_val) * 100
        else:  # For TPS, higher is better
            regression_pct = ((baseline_val - current_val) / baseline_val) * 100
        
        print(f"{metric}: {current_val} (baseline: {baseline_val}, delta: {regression_pct:.1f}%)")
        
        if regression_pct > threshold_pct:
            print(f"❌ REGRESSION: {metric} degraded by {regression_pct:.1f}%")
            sys.exit(1)
    
    print("✅ All metrics within acceptable range")

if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument('--current', required=True)
    parser.add_argument('--baseline', required=True)
    parser.add_argument('--threshold', type=float, default=10)
    args = parser.parse_args()
    
    compare(args.current, args.baseline, args.threshold)
```

---

## 10. Best Practices

### 10.1 Writing Integration Tests

**DO:**
- ✅ Use `testcontainers-rs` for isolated PostgreSQL/Redis/Anvil instances
- ✅ Test one scenario per test function (focused assertions)
- ✅ Clean up resources in `Drop` implementations
- ✅ Use descriptive test names (`concurrent_nonce_requests_are_sequential`)

**DON'T:**
- ❌ Share state between tests (leads to flaky failures)
- ❌ Hard-code timeouts (use `tokio::time::timeout` with generous limits)
- ❌ Ignore cleanup (leftover containers waste CI resources)

### 10.2 Benchmark Execution

**DO:**
- ✅ Run benchmarks on dedicated hardware (no shared CI runners)
- ✅ Disable CPU frequency scaling (`sudo cpupower frequency-set --governor performance`)
- ✅ Close background applications (browser, Slack, etc.)
- ✅ Run multiple iterations (3-5) and report median
- ✅ Store raw Prometheus data for historical analysis

**DON'T:**
- ❌ Run benchmarks during active development (code churn invalidates comparisons)
- ❌ Compare across different hardware (absolute numbers aren't portable)
- ❌ Cherry-pick best results (report all runs, flag outliers)

### 10.3 Metric Collection

**DO:**
- ✅ Use histogram buckets appropriate to your latency range (0.001 - 5.0 seconds)
- ✅ Label metrics by stage/chain/error type for drill-down
- ✅ Export both rate (TPS) and cumulative counters (total transactions)

**DON'T:**
- ❌ Use high-cardinality labels (e.g., `execution_id` — explodes Prometheus memory)
- ❌ Scrape more frequently than 15s (unnecessary load on Prometheus)

### 10.4 Result Interpretation

**DO:**
- ✅ Focus on P95/P99 latency (P50 hides tail behavior)
- ✅ Compare error rates, not just throughput (10,000 TPS with 50% errors is useless)
- ✅ Check for memory leaks (monitor Lobby RSS over long runs)

**DON'T:**
- ❌ Optimize for synthetic benchmarks (real-world traffic is bursty, not steady)
- ❌ Ignore variance (a benchmark with stddev > 20% needs investigation)

---

## 11. Appendices

### 11.1 Sample Benchmark Report

```
============================================================
           Lobby Benchmark Report
============================================================
Date:        2026-03-22 14:32:01 UTC
Profile:     steady_1000tps.toml
Duration:    60 seconds
Lobby Ver:   0.1.0

------------------------------------------------------------
Configuration
------------------------------------------------------------
Target TPS:                1,000
Chain:                     Anvil (31337)
Block Time:                1 second
Test Accounts:             100
Pipeline Concurrency:      17
Nonce Shards:              17
Sign Shards:               17
Broadcast Shards:          17

------------------------------------------------------------
Results Summary
------------------------------------------------------------
✅ Achieved TPS:            1,023  (target: 1,000)
✅ Total Transactions:      61,380
✅ Successful:              61,368 (99.98%)
❌ Failed:                  12     (0.02%)
✅ Nonce Conflicts:         0

------------------------------------------------------------
Latency (End-to-End)
------------------------------------------------------------
P50:     3.2s
P95:     4.8s  ✅ (target: < 5s)
P99:     6.1s
Max:     8.3s

------------------------------------------------------------
Latency Breakdown (P95)
------------------------------------------------------------
Submission:      47ms
Nonce Reserve:   102ms
Sign:            51ms
Broadcast:       189ms
Validator:       4,211ms

------------------------------------------------------------
Error Analysis
------------------------------------------------------------
Broadcast timeout:   8 (RPC node overload)
Nonce reservation:   3 (DB connection spike)
Validator timeout:   1 (block time variance)

------------------------------------------------------------
Resource Usage
------------------------------------------------------------
Lobby Peak RSS:      847 MB
PostgreSQL Conn:     17/17 (pool saturated)
Prometheus Size:     1.2 GB (60 min retention)

------------------------------------------------------------
Recommendations
------------------------------------------------------------
1. Increase Validator poll_interval (2s → 1s) to reduce confirmation latency
2. Monitor PostgreSQL connection pool (consider increasing to 25)
3. Investigate broadcast timeouts (Anvil may need higher gas limit)

============================================================
```

### 11.2 PromQL Query Cheat Sheet

```promql
# Throughput (TPS)
rate(lobby_requests_total[1m])

# P95 latency
histogram_quantile(0.95, rate(lobby_pipeline_stage_duration_seconds_bucket[5m]))

# Error rate (%)
(rate(lobby_errors_total[1m]) / rate(lobby_requests_total[1m])) * 100

# Active pipelines (semaphore usage)
lobby_pipeline_semaphore_active

# Database query latency
rate(lobby_db_query_duration_seconds_sum[1m]) / rate(lobby_db_query_duration_seconds_count[1m])

# Actor shard load imbalance (max vs. min)
max(rate(lobby_nonce_requests_total[1m])) - min(rate(lobby_nonce_requests_total[1m]))
```

### 11.3 Troubleshooting Guide

| Symptom | Likely Cause | Solution |
|---------|--------------|----------|
| TPS plateaus at 500 | Semaphore exhausted | Increase `PIPELINE_CONCURRENCY` |
| P95 latency > 10s | Validator timeout | Check Anvil block production, reduce `required_confirmations` |
| Nonce conflicts (duplicates) | Actor shard collision | Verify `ByAddress` hashing is deterministic |
| Broadcast failures spike | RPC rate limiting | Add more Anvil instances, load balance |
| PostgreSQL connection errors | Pool exhausted | Increase `max_connections` in `PgPoolOptions` |
| Memory leak (RSS grows) | StatusRegistry unbounded | Implement TTL-based eviction in Redis |

### 11.4 Future Enhancements

**Planned for Entropy v0.2:**
- [ ] WebSocket-based validator (eliminate polling latency)
- [ ] Multi-region Anvil clusters (geo-distributed load testing)
- [ ] Automated performance regression alerts (Slack/PagerDuty)
- [ ] Comparative benchmarks (Lobby vs. Flashbots Protect vs. Alchemy Transact)
- [ ] Chaos engineering suite (Toxiproxy for network faults)

**Contributions Welcome:**
- Load generator plugins (custom transaction types)
- Alternative monitor stacks (Datadog, New Relic)
- Real-world transaction replay (from Ethereum mempool dumps)

---
## License

This testing strategy is part of the Lobby project, licensed under Apache-2.0.

---
**End of Testing & Benchmarking Strategy Document**

*Built with Rust, Tokio, Testcontainers, Prometheus, and Grafana.*  
*Designed to validate Lobby's promise: 1000 TPS, sub-5s latency, zero nonce conflicts.*
