# Lobby Testing & Benchmarking Strategy Guide

**Version:** 0.1.0 (Initial Testing Infrastructure)  
**Last Updated:** Mar 02, 2026  
**Target Audience:** Contributors, maintainers, and LLMs working with Lobby's test infrastructure  
**Prerequisites:** Read [Lobby_Dev_Doc.md](../../ensemble/docs/Lobby_Dev_Doc.md) and [Client_API_Doc.md](../../ensemble/docs/Client_API_Doc.md) first

---

## Table of Contents

1. [Introduction](#introduction)
2. [Testing Philosophy](#testing-philosophy)
3. [Test Infrastructure Overview](#test-infrastructure-overview)
4. [Test Layers & Responsibilities](#test-layers--responsibilities)
5. [Test Environment Architecture](#test-environment-architecture)
6. [Database Testing Strategy](#database-testing-strategy)
7. [Blockchain Testing Strategy](#blockchain-testing-strategy)
8. [Test Data & Key Management](#test-data--key-management)
9. [Failure Mode Coverage Matrix](#failure-mode-coverage-matrix)
10. [Benchmarking Strategy](#benchmarking-strategy)
11. [CI/CD Pipeline Architecture](#cicd-pipeline-architecture)
12. [Directory Structure & Organization](#directory-structure--organization)
13. [Running Tests Locally](#running-tests-locally)
14. [Writing New Tests](#writing-new-tests)
15. [Debugging Failed Tests](#debugging-failed-tests)
16. [Performance Regression Detection](#performance-regression-detection)
17. [Future Enhancements](#future-enhancements)

---

## Introduction

This document defines the **comprehensive testing, benchmarking, and continuous integration strategy** for Lobby, a high-throughput custodial EVM transaction signing service. 

Lobby's architecture (detailed in [Lobby_Dev_Doc.md](./Lobby_Dev_Doc.md)) is built on an **actor-based concurrency model** with strict correctness guarantees around nonce ordering, idempotency, and state consistency. Testing such a system requires a multi-layered approach that validates:

- **Correctness:** Nonce monotonicity, TOCTOU-safe queries, lease-based idempotency
- **Concurrency Safety:** Race-free nonce assignment across 17 shards, actor message passing integrity
- **Failure Recovery:** Retry logic, nonce release on pipeline failure, actor panic handling
- **Performance:** Sub-100ms pipeline latency (internal), 1000+ TPS throughput
- **Integration:** Real database transactions, blockchain RPC interactions, end-to-end workflows

### Why This Document Exists

Unlike traditional web services, Lobby combines:
- **Distributed state** across 5 actor types (RelayHost, Nonce, Sign, Broadcast, Validator)
- **External dependencies** with non-deterministic behavior (PostgreSQL, blockchain RPCs)
- **Financial correctness requirements** (nonce conflicts = stuck transactions, lost funds)
- **High-concurrency edge cases** (17 concurrent pipelines, sharded actors, lease expiration races)

Standard unit testing is **insufficient**. We need:
- **Property-based tests** to verify invariants across random inputs
- **Integration tests** with real databases to catch TOCTOU bugs
- **Chaos tests** to simulate RPC failures, database disconnects, actor panics
- **Benchmarks** to detect performance regressions and measure scalability

This document is the **single source of truth** for how we validate Lobby's correctness and performance.

---

## Testing Philosophy

### Core Principles

**1. Correctness Before Performance**
- A fast but incorrect system is worthless
- All optimizations must preserve nonce ordering, idempotency, and state consistency
- Tests that verify correctness invariants have the highest priority

**2. Test What Matters**
- Focus on **failure modes that cause real bugs** (nonce conflicts, transaction reversion, actor panics)
- Avoid testing implementation details (e.g., exact log message format)
- Prefer testing observable behavior (database state, API responses, blockchain transactions)

**3. Real Dependencies Where It Matters**
- **Always use real PostgreSQL** (via `testcontainers`) — mocking SQLx is too risky
- **Always use real blockchain nodes** (Anvil forks) — RPC quirks are part of the system
- **Mock only when determinism matters** (e.g., RPC timeouts in unit tests)

**4. Deterministic Tests**
- Tests must pass 100% of the time (no flaky tests tolerated)
- Use event-driven synchronization, not sleeps
- Generate fresh UUIDs per test (no hardcoded `execution_id` reuse)

**5. Fast Feedback Loops**
- Unit tests: < 5 seconds total
- Integration tests: < 2 minutes total
- E2E tests: < 10 minutes total
- Benchmarks: Run on-demand, not in CI (too slow)

**6. Production-Like Testing**
- Test databases use the same SQLx migrations as production
- Anvil forks replicate real Ethereum behavior (gas estimation, reversion)
- Test keys follow the same `test_keys.json` format as production

### What We Test vs. What We Don't

| ✅ **Test This** | ❌ **Don't Test This** |
|---|---|
| Nonce monotonicity across concurrent requests | Log message exact wording |
| TOCTOU-safe query atomicity | Actor mpsc channel buffer size |
| Lease expiration edge cases (exactly 5 minutes) | Retry backoff jitter exact milliseconds |
| Transaction reversion on-chain | JSON-RPC serialization format (trust Alloy) |
| Actor panic recovery (3 retries, then crash) | Environment variable parsing logic |
| Pipeline status state transitions | HTTP routing (trust Axum) |
| Database trigger correctness (`updated_at` auto-update) | Tracing span hierarchy |

---

## Test Infrastructure Overview

### The 6-Layer Testing Pyramid

```
                        ┌──────────────────┐
                        │  E2E Tests       │  ← Full system + Anvil blockchain
                        │  (10 tests)      │
                        └──────────────────┘
                      ┌────────────────────────┐
                      │  Chaos Tests           │  ← Fault injection (DB failures, RPC timeouts)
                      │  (15 tests)            │
                      └────────────────────────┘
                   ┌───────────────────────────────┐
                   │  Property Tests               │  ← Invariant verification (nonce ordering)
                   │  (20 tests)                   │
                   └───────────────────────────────┘
                ┌────────────────────────────────────┐
                │  Integration Tests                 │  ← Multi-actor, real DB, mock RPC
                │  (50 tests)                        │
                └────────────────────────────────────┘
             ┌───────────────────────────────────────────┐
             │  Component Tests                          │  ← Single actor, mock deps
             │  (100 tests)                              │
             └───────────────────────────────────────────┘
          ┌──────────────────────────────────────────────────┐
          │  Unit Tests                                      │  ← Pure functions, no I/O
          │  (150 tests)                                     │
          └──────────────────────────────────────────────────┘
```

**Test counts are targets, not hard limits.** Quality > quantity.

### Technology Stack

| Layer | Primary Tools | Purpose |
|-------|--------------|---------|
| **Unit Tests** | `cargo test`, `rstest` | Fast, deterministic, no I/O |
| **Component Tests** | `tokio::test`, `mockall`, `testcontainers` | Single actor with real DB |
| **Integration Tests** | `testcontainers`, `wiremock` | Multi-actor coordination |
| **Property Tests** | `proptest`, `quickcheck` | Invariant verification via fuzzing |
| **Chaos Tests** | `testcontainers`, `toxiproxy` (optional) | Fault injection |
| **E2E Tests** | `reqwest`, `foundry-rs/anvil` | Full system + blockchain |
| **Benchmarks** | `criterion`, `cargo-flamegraph` | Performance profiling |
| **CI/CD** | GitLab CI, Docker, `cargo-nextest` | Automated testing & gating |

---

## Test Layers & Responsibilities

### Layer 1: Unit Tests

**What:** Pure functions, type conversions, error handling logic  
**No I/O:** No database, no network, no filesystem  
**Location:** `entropy/unit-tests/`

**Examples:**
- `ExecutionId::from_bytes()` round-trip correctness
- `RetryConfig::default()` values match specification
- `jittered_delay()` returns values within `[0, max_delay]`
- Error type conversions (`BroadcastError -> OrchestratorError`)

**Execution Time:** < 100ms for entire suite

**Sample Test:**
```rust
#[test]
fn execution_id_roundtrip() {
    let original = ExecutionId::new_v4();
    let bytes = original.as_bytes();
    let recovered = ExecutionId::from_bytes(bytes).unwrap();
    assert_eq!(original, recovered);
}
```

---

### Layer 2: Component Tests

**What:** Single actor behavior with mocked dependencies  
**Real Database:** Yes (via `testcontainers`)  
**Real RPC:** No (mocked with `wiremock`)  
**Location:** `entropy/component-tests/tests/`

**Focus Areas:**
- **Nonce Actor:** Reserve/resolve logic, lease expiration, concurrent requests
- **Sign Actor:** Signature correctness, key lookup, audit trail
- **Broadcast Actor:** RPC submission, retry on timeout, rejection handling
- **Validator Actor:** Receipt polling, confirmation depth, reorg detection
- **RelayHost:** Intent persistence, idempotency, validation

**Example Test Structure:**
```rust
#[tokio::test]
async fn nonce_actor_prevents_duplicate_reservation() {
    // Setup: Start Postgres in Docker
    let container = testcontainers::runners::AsyncRunner::default()
        .start(testcontainers_modules::postgres::Postgres::default())
        .await;
    
    let db = PgPool::connect(&connection_string).await.unwrap();
    sqlx::migrate!("../../ensemble/database/migrations").run(&db).await.unwrap();
    
    // Spawn nonce actor
    let nonce_handle = spawn_nonce_actor(db.clone(), 64);
    
    let execution_id = ExecutionId::new_v4();
    let chain_id = ChainId(1);
    let from = Address::random();
    
    // First reservation succeeds
    let nonce1 = nonce_handle.reserve(chain_id, from, execution_id).await.unwrap();
    
    // Duplicate reservation within 5-minute lease fails
    let result = nonce_handle.reserve(chain_id, from, execution_id).await;
    assert!(matches!(result, Err(LocalError::AlreadyReserved)));
    
    // Verify database state: only one 'reserved' row exists
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM nonce.nonce_assignments WHERE execution_id = $1 AND state = 'reserved'"
    )
    .bind(execution_id.as_bytes())
    .fetch_one(&db)
    .await
    .unwrap();
    
    assert_eq!(count, 1);
}
```

---

### Layer 3: Integration Tests

**What:** Multi-actor orchestration, pipeline workflows  
**Real Database:** Yes (via `testcontainers`)  
**Real RPC:** No (mocked with `wiremock`)  
**Location:** `entropy/integration-tests/tests/`

**Critical Tests:**

**1. Pipeline Happy Path**
- Submit transaction → RelayHost → Nonce → Sign → Broadcast → Validator → Status = "confirmed"
- Verify nonce is finalized, database audit trail complete

**2. Pipeline Failure Recovery**
- Broadcast fails after 3 retries → nonce is released
- Sign actor panics → nonce is released, status = "failed"

**3. Concurrent Nonce Safety**
- Submit 100 transactions for same address simultaneously
- Verify nonces are 0, 1, 2, ..., 99 (no gaps, no duplicates)

**4. Idempotency Enforcement**
- Submit same `execution_id` twice within 5 minutes → second request rejected
- Submit same `execution_id` after 6 minutes → lease expired, treated as new request

**Example Test:**
```rust
#[tokio::test]
async fn pipeline_releases_nonce_on_broadcast_failure() {
    // Setup test environment
    let (db, mock_rpc_server) = setup_test_environment().await;
    
    // Configure RPC mock to always fail
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(500))
        .expect(3) // Will be called 3 times (initial + 2 retries)
        .mount(&mock_rpc_server)
        .await;
    
    // Spawn orchestrator
    let orchestrator = spawn_test_orchestrator(db.clone(), &mock_rpc_server.uri()).await;
    
    let execution_id = ExecutionId::new_v4();
    let txn = build_test_transaction();
    let client_config = build_test_client_config();
    
    // Submit transaction
    orchestrator.submit(execution_id, txn, client_config).await.unwrap();
    
    // Wait for pipeline to complete
    wait_for_terminal_status(&orchestrator, execution_id, Duration::from_secs(10)).await;
    
    // Verify status is "failed" at broadcast stage
    let status = orchestrator.get_status(execution_id).unwrap();
    assert_eq!(status, PipelineStatus::Failed {
        stage: "broadcast".to_string(),
        reason: "RPC request failed after 3 attempts".to_string(),
    });
    
    // CRITICAL: Verify nonce was released
    let nonce_state: String = sqlx::query_scalar(
        "SELECT state::text FROM nonce.nonce_assignments 
         WHERE execution_id = $1 
         ORDER BY revision DESC LIMIT 1"
    )
    .bind(execution_id.as_bytes())
    .fetch_one(&db)
    .await
    .unwrap();
    
    assert_eq!(nonce_state, "released");
}
```

---

### Layer 4: Property Tests

**What:** Verify invariants hold across randomized inputs  
**Tools:** `proptest` for fuzzing, `quickcheck` for simpler cases  
**Location:** `entropy/property-tests/tests/`

**Invariants to Test:**

**1. Nonce Monotonicity**
- **Property:** For any address, nonces assigned across N concurrent requests must be `[0, 1, 2, ..., N-1]` (no gaps, no duplicates)
- **Strategy:** Generate random number of requests (1-100), random addresses, submit concurrently, verify order

**2. Lease Expiration Safety**
- **Property:** If a nonce is reserved at time T, a duplicate `execution_id` request at time T+6min must succeed (lease expired)
- **Strategy:** Fuzz reservation times, fast-forward clock (via `tokio::time::pause`), verify behavior

**3. State Machine Transitions**
- **Property:** Pipeline status can only transition through valid states (no `accepted → confirmed` without `nonce_reserved → signed → broadcast`)
- **Strategy:** Generate random pipeline event sequences, verify state transitions are legal

**4. Idempotency Under Retry**
- **Property:** If a request is retried N times, the database must contain exactly 1 final state (not N duplicate rows)
- **Strategy:** Fuzz retry counts, verify row count invariant

**Example Property Test:**
```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn nonce_monotonicity_across_concurrent_requests(
        num_requests in 1usize..100,
        from_address in any::<[u8; 20]>().prop_map(Address::from),
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (db, nonce_handle) = setup_nonce_actor().await;
            
            let chain_id = ChainId(1);
            let mut handles = vec![];
            
            // Spawn concurrent reservation requests
            for _ in 0..num_requests {
                let handle = nonce_handle.clone();
                let exec_id = ExecutionId::new_v4();
                
                handles.push(tokio::spawn(async move {
                    handle.reserve(chain_id, from_address, exec_id).await
                }));
            }
            
            // Collect assigned nonces
            let mut nonces: Vec<TxNonce> = handles
                .into_iter()
                .map(|h| rt.block_on(h).unwrap().unwrap())
                .collect();
            
            nonces.sort();
            
            // Verify monotonicity: [0, 1, 2, ..., num_requests-1]
            let expected: Vec<TxNonce> = (0..num_requests as u64).collect();
            prop_assert_eq!(nonces, expected);
        });
    }
}
```

---

### Layer 5: Chaos Tests

**What:** Fault injection to verify graceful degradation  
**Tools:** `testcontainers`, `wiremock` (for controlled failures), `toxiproxy` (optional, for network faults)  
**Location:** `entropy/chaos-tests/tests/`

**Failure Scenarios:**

**1. Database Connection Failures**
- Kill Postgres container mid-transaction → verify retry logic
- Exhaust connection pool → verify backpressure (semaphore timeout)

**2. RPC Provider Failures**
- Timeout on `eth_sendRawTransaction` → verify retry + nonce release
- Rate limit (429 error) → verify exponential backoff
- Invalid response (malformed JSON) → verify error handling

**3. Actor Panics**
- Force Sign actor to panic → verify pipeline fails after 3 retries
- Force Nonce actor to panic → verify system crash (no silent failures)

**4. Lease Expiration Races**
- Reserve nonce, sleep 5 minutes, verify lease expired
- Concurrent request arrives at exactly 5:00.000 boundary → verify one succeeds, one fails

**Example Chaos Test:**
```rust
#[tokio::test]
async fn database_connection_loss_triggers_retry() {
    let container = testcontainers::runners::AsyncRunner::default()
        .start(testcontainers_modules::postgres::Postgres::default())
        .await;
    
    let db = PgPool::connect(&connection_string).await.unwrap();
    sqlx::migrate!("../../ensemble/database/migrations").run(&db).await.unwrap();
    
    let nonce_handle = spawn_nonce_actor(db.clone(), 64);
    
    // First reservation succeeds
    let exec_id = ExecutionId::new_v4();
    let nonce = nonce_handle.reserve(ChainId(1), Address::random(), exec_id).await.unwrap();
    
    // Kill the database
    drop(container);
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Attempt to finalize nonce → should fail, but retries kick in
    let result = nonce_handle.resolve(exec_id, true).await;
    
    // After 3 retries, we expect failure (no database to connect to)
    assert!(matches!(result, Err(LocalError::DatabaseError(_))));
}
```

---

### Layer 6: End-to-End Tests

**What:** Full system with real HTTP server, database, and blockchain  
**Real Database:** Yes (via `testcontainers`)  
**Real Blockchain:** Yes (Anvil fork in Docker)  
**Location:** `entropy/e2e-tests/tests/`

**Test Scenarios:**

**1. Submit Transaction via HTTP → Poll Status → Confirm On-Chain**
- Start Lobby server, Postgres, Anvil
- POST to `/v1/transactions` → receive `execution_id`
- Poll `GET /status/{execution_id}` until status = "confirmed"
- Verify transaction exists on Anvil blockchain (`eth_getTransactionReceipt`)

**2. ERC-20 Token Transfer**
- Deploy mock ERC-20 contract on Anvil
- Submit `transfer()` transaction via Lobby
- Verify recipient balance increased

**3. Uniswap V2 Swap**
- Fork Hoodi with Uniswap V2 contracts
- Submit `swapExactTokensForTokens()` transaction
- Verify swap executed on-chain

**4. Contract Deployment**
- Submit transaction with `to: null`, `data: <bytecode>`
- Verify contract deployed at expected address

**5. Transaction Reversion**
- Submit transaction that will revert (e.g., insufficient balance, bad function call)
- Verify Lobby detects reversion, status = "failed"
- Verify nonce was released (can reuse for next transaction)

**Example E2E Test:**
```rust
#[tokio::test]
async fn e2e_submit_transaction_and_confirm_on_chain() {
    // Start Anvil (Ethereum fork)
    let anvil = Anvil::new()
        .fork("https://eth-hoodi.g.alchemy.com/v2/...")
        .spawn();
    
    // Start Postgres
    let pg_container = testcontainers::runners::AsyncRunner::default()
        .start(testcontainers_modules::postgres::Postgres::default())
        .await;
    
    let db_url = format!("postgres://postgres:postgres@localhost:{}/postgres", pg_container.get_host_port_ipv4(5432).await);
    
    // Start Lobby server
    let lobby_server = spawn_lobby_server(db_url, anvil.endpoint()).await;
    
    // Fund test account on Anvil
    let from_address = anvil.addresses()[0];
    fund_account(&anvil, from_address, parse_ether("10").unwrap()).await;
    
    // Submit transaction via HTTP
    let client = reqwest::Client::new();
    let response: SubmitResponse = client
        .post(format!("{}/v1/transactions", lobby_server.url()))
        .header("Authorization", "Bearer test_api_key")
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "eth_sendTransaction",
            "params": [{
                "from": format!("{:#x}", from_address),
                "to": "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
                "value": "0xde0b6b3a7640000", // 1 ETH
                "chainId": "0x4269", // Hoodi
                "gas": "0x5208",
                "maxFeePerGas": "0xba43b7400",
                "maxPriorityFeePerGas": "0x77359400",
            }],
            "id": 1,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    
    let execution_id = response.result.execution_id;
    
    // Poll status until confirmed
    let final_status = poll_until_terminal(&client, &lobby_server.url(), execution_id, Duration::from_secs(60)).await;
    
    assert_eq!(final_status.status, "confirmed");
    let tx_hash = final_status.tx_hash.unwrap();
    
    // Verify transaction on Anvil
    let provider = Provider::<Http>::try_from(anvil.endpoint()).unwrap();
    let receipt = provider.get_transaction_receipt(tx_hash).await.unwrap().unwrap();
    
    assert_eq!(receipt.status, Some(1.into())); // Success
    assert_eq!(receipt.from, from_address);
}
```

---

## Test Environment Architecture

### Docker Compose for Local Testing

All tests use **ephemeral Docker containers** to ensure isolation and repeatability.

**Services:**
- **PostgreSQL 16** (with shared volumes for migrations)
- **Anvil** (Foundry's local Ethereum node, forked from Hoodi)
- **Optional: Toxiproxy** (for network fault injection)

**Why Docker?**
- **Deterministic environment:** Same Postgres version, same Anvil behavior
- **Fast setup/teardown:** `testcontainers` starts/stops containers per test
- **CI/CD compatibility:** GitLab CI runs Docker natively

**Architecture Diagram:**
```
┌─────────────────────────────────────────────────────────┐
│  Test Process (Rust)                                    │
│                                                          │
│  ┌──────────────┐   ┌──────────────┐   ┌─────────────┐ │
│  │ Component    │   │ Integration  │   │  E2E Test   │ │
│  │ Test         │   │ Test         │   │             │ │
│  └──────┬───────┘   └──────┬───────┘   └──────┬──────┘ │
│         │                  │                   │        │
│         ▼                  ▼                   ▼        │
│  ┌──────────────────────────────────────────────────┐  │
│  │  testcontainers-rs                              │  │
│  └──────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
         │                  │                   │
         ▼                  ▼                   ▼
┌─────────────────────────────────────────────────────────┐
│  Docker Host                                            │
│                                                          │
│  ┌──────────────┐   ┌──────────────┐   ┌─────────────┐ │
│  │ Postgres     │   │ Anvil        │   │ Toxiproxy   │ │
│  │ (Port 5432)  │   │ (Port 8545)  │   │ (Optional)  │ │
│  └──────────────┘   └──────────────┘   └─────────────┘ │
│                                                          │
│  Auto-started/stopped per test                          │
└─────────────────────────────────────────────────────────┘
```

---

## Database Testing Strategy

### Migration Management

**Key Principle:** Test databases use the **exact same migrations** as production.

**Location:** `ensemble/database/migrations/` (shared across test and prod)

**Migration Tool:** `sqlx-cli`

**Test Setup Pattern:**
```rust
async fn setup_test_db() -> PgPool {
    let container = testcontainers::runners::AsyncRunner::default()
        .start(testcontainers_modules::postgres::Postgres::default())
        .await;
    
    let port = container.get_host_port_ipv4(5432).await;
    let db_url = format!("postgres://postgres:postgres@localhost:{}/postgres", port);
    
    let pool = PgPool::connect(&db_url).await.unwrap();
    
    // Apply production migrations
    sqlx::migrate!("../../ensemble/database/migrations")
        .run(&pool)
        .await
        .unwrap();
    
    pool
}
```

### Schema Validation Tests

**Purpose:** Ensure migrations create correct schema (indexes, triggers, constraints)

**Examples:**
```rust
#[tokio::test]
async fn nonce_schema_has_unique_active_index() {
    let db = setup_test_db().await;
    
    // Verify partial unique index exists
    let index_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM pg_indexes 
            WHERE schemaname = 'nonce' 
            AND indexname = 'uniq_active_nonce'
        )"
    )
    .fetch_one(&db)
    .await
    .unwrap();
    
    assert!(index_exists, "Missing critical nonce uniqueness index");
}

#[tokio::test]
async fn relay_host_prevents_updates() {
    let db = setup_test_db().await;
    
    // Insert transaction intent
    let execution_id = ExecutionId::new_v4();
    sqlx::query(
        "INSERT INTO relay_host.transaction_intents 
         (execution_id, client_id, chain_id, from_address, value, data, gas_limit, max_fee_per_gas, max_priority_fee_per_gas)
         VALUES ($1, $2, 1, $3, 0, $4, 21000, 50000000000, 2000000000)"
    )
    .bind(execution_id.as_bytes())
    .bind(Uuid::new_v4().as_bytes())
    .bind([0u8; 20].as_ref())
    .bind([0u8; 0].as_ref())
    .execute(&db)
    .await
    .unwrap();
    
    // Attempt to update (should fail due to trigger)
    let result = sqlx::query(
        "UPDATE relay_host.transaction_intents SET value = 1 WHERE execution_id = $1"
    )
    .bind(execution_id.as_bytes())
    .execute(&db)
    .await;
    
    assert!(result.is_err(), "Relay host trigger should prevent updates");
}
```

### TOCTOU Query Tests

**Critical:** Verify that `INSERT ... SELECT ... WHERE NOT EXISTS` queries are truly atomic.

**Test Pattern:**
```rust
#[tokio::test]
async fn nonce_reservation_is_toctou_safe() {
    let db = setup_test_db().await;
    
    let execution_id = ExecutionId::new_v4();
    let chain_id = ChainId(1);
    let from_address = Address::random();
    
    // Simulate race: Two concurrent connections try to reserve same nonce
    let db1 = db.clone();
    let db2 = db.clone();
    
    let handle1 = tokio::spawn(async move {
        reserve_nonce_query(&db1, execution_id, chain_id, from_address).await
    });
    
    let handle2 = tokio::spawn(async move {
        reserve_nonce_query(&db2, execution_id, chain_id, from_address).await
    });
    
    let (result1, result2) = tokio::join!(handle1, handle2);
    
    // Exactly one should succeed (partial unique index prevents both)
    let success_count = [result1, result2].iter().filter(|r| r.is_ok()).count();
    assert_eq!(success_count, 1, "TOCTOU violation: both reservations succeeded");
}
```

---

## Blockchain Testing Strategy

### Anvil Fork Configuration

**Why Anvil?**
- **Deterministic:** Local node, no external dependencies
- **Fast:** Instant block mining, no waiting for confirmations
- **Forkable:** Can fork from Hoodi to test against real contracts (Uniswap, ERC-20s)
- **Free:** No RPC rate limits

**Setup:**
```rust
use foundry_config::Config;

async fn start_anvil_fork() -> AnvilInstance {
    Anvil::new()
        .fork("https://eth-hoodi.g.alchemy.com/v2/API_KEY")
        .fork_block_number(5_000_000) // Pin to specific block for reproducibility
        .chain_id(17001) // Hoodi chain ID
        .block_time(1) // 1-second block time (faster than real Hoodi)
        .spawn()
}
```

**Docker Alternative (for CI):**
```yaml
# docker-compose.test.yml
services:
  anvil:
    image: ghcr.io/foundry-rs/foundry:latest
    command: >
      anvil
      --fork-url https://eth-hoodi.g.alchemy.com/v2/API_KEY
      --fork-block-number 5000000
      --chain-id 17001
      --block-time 1
      --host 0.0.0.0
    ports:
      - "8545:8545"
```

### Test Account Funding

**Strategy:** Use Anvil's `anvil_setBalance` RPC to fund ephemeral test accounts (no faucet needed).

```rust
async fn fund_account(anvil: &AnvilInstance, address: Address, amount: U256) {
    let provider = Provider::<Http>::try_from(anvil.endpoint()).unwrap();
    
    provider
        .request(
            "anvil_setBalance",
            (format!("{:#x}", address), format!("{:#x}", amount)),
        )
        .await
        .unwrap();
}
```

### Realistic Transaction Templates

**1. Simple ETH Transfer**
```rust
fn build_eth_transfer(from: Address, to: Address, value: U256) -> Eip1559Transaction {
    Eip1559Transaction {
        chain_id: ChainId(17001),
        nonce: 0, // Will be assigned by Nonce actor
        max_priority_fee_per_gas: U256::from(2_000_000_000u64), // 2 gwei
        max_fee_per_gas: U256::from(50_000_000_000u64), // 50 gwei
        gas_limit: U256::from(21000u64),
        to: Some(to),
        value,
        data: Bytes::new(),
        access_list: vec![],
    }
}
```

**2. ERC-20 Transfer**
```rust
fn build_erc20_transfer(
    token_address: Address,
    from: Address,
    to: Address,
    amount: U256,
) -> Eip1559Transaction {
    // ABI-encode: transfer(address,uint256)
    let selector = &keccak256(b"transfer(address,uint256)")[..4];
    let mut data = Vec::from(selector);
    data.extend_from_slice(&[0u8; 12]); // Pad address to 32 bytes
    data.extend_from_slice(to.as_bytes());
    data.extend_from_slice(&amount.to_be_bytes::<32>());
    
    Eip1559Transaction {
        chain_id: ChainId(17001),
        nonce: 0,
        max_priority_fee_per_gas: U256::from(2_000_000_000u64),
        max_fee_per_gas: U256::from(50_000_000_000u64),
        gas_limit: U256::from(55000u64), // ERC-20 transfer gas
        to: Some(token_address),
        value: U256::ZERO,
        data: Bytes::from(data),
        access_list: vec![],
    }
}
```

**3. Uniswap V2 Swap (requires Hoodi fork)**
```rust
fn build_uniswap_swap(
    router_address: Address,
    amount_in: U256,
    amount_out_min: U256,
    path: Vec<Address>, // [token_in, token_out]
    deadline: U256,
) -> Eip1559Transaction {
    // ABI-encode: swapExactTokensForTokens(uint256,uint256,address[],address,uint256)
    let selector = &keccak256(b"swapExactTokensForTokens(uint256,uint256,address[],address,uint256)")[..4];
    // ... (full ABI encoding omitted for brevity)
    
    Eip1559Transaction {
        chain_id: ChainId(17001),
        nonce: 0,
        max_priority_fee_per_gas: U256::from(2_000_000_000u64),
        max_fee_per_gas: U256::from(100_000_000_000u64), // Higher gas for complex tx
        gas_limit: U256::from(200000u64),
        to: Some(router_address),
        value: U256::ZERO,
        data: Bytes::from(encoded_data),
        access_list: vec![],
    }
}
```

### Transaction Reversion Tests

**Test Scenario:** Submit transaction that will revert on-chain → verify Lobby handles gracefully.

**Examples:**
- **Insufficient balance:** Transfer more ETH than account owns
- **ERC-20 revert:** Transfer tokens without approval
- **Custom contract revert:** Call function that throws

```rust
#[tokio::test]
async fn validator_detects_transaction_reversion() {
    let anvil = start_anvil_fork().await;
    let from_address = anvil.addresses()[0];
    
    // Fund account with only 1 ETH
    fund_account(&anvil, from_address, parse_ether("1").unwrap()).await;
    
    // Try to send 10 ETH (will fail)
    let txn = build_eth_transfer(
        from_address,
        Address::random(),
        parse_ether("10").unwrap(), // Insufficient balance
    );
    
    let lobby = spawn_lobby_with_anvil(&anvil).await;
    let execution_id = ExecutionId::new_v4();
    
    lobby.submit(execution_id, txn, test_client_config()).await.unwrap();
    
    // Wait for pipeline to complete
    let status = poll_until_terminal(&lobby, execution_id, Duration::from_secs(30)).await;
    
    assert_eq!(status, PipelineStatus::Failed {
        stage: "validator".to_string(),
        reason: "transaction reverted on-chain (receipt.status = 0)".to_string(),
    });
    
    // Verify nonce was released
    verify_nonce_released(&lobby.db, execution_id).await;
}
```

---

## Test Data & Key Management

### Ephemeral Test Key Generation

**Strategy:** Generate fresh keys per test run, fund via Anvil's `anvil_setBalance`.

**Number of Accounts:** **34 test accounts** (2× the shard count for balanced testing)
- 17 accounts for shard coverage (one per Nonce shard)
- 17 additional accounts for concurrent testing

**Key Generation:**
```rust
use alloy_signer::k256::ecdsa::SigningKey;

fn generate_test_keys(count: usize) -> Vec<TestAccount> {
    (0..count)
        .map(|i| {
            let private_key = SigningKey::random(&mut rand::thread_rng());
            let address = /* derive address from public key */;
            
            TestAccount {
                index: i,
                private_key: format!("0x{}", hex::encode(private_key.to_bytes())),
                address: format!("{:#x}", address),
            }
        })
        .collect()
}
```

**Test Keys Fixture File:**
```rust
// entropy/test-utils/src/fixtures.rs

lazy_static! {
    pub static ref TEST_KEYS: Vec<TestAccount> = generate_test_keys(34);
}

pub struct TestAccount {
    pub index: usize,
    pub private_key: String,
    pub address: String,
}

impl TestAccount {
    pub fn for_shard(shard: usize, total_shards: usize) -> &'static Self {
        &TEST_KEYS[shard % TEST_KEYS.len()]
    }
    
    pub fn random() -> &'static Self {
        &TEST_KEYS[rand::random::<usize>() % TEST_KEYS.len()]
    }
}
```

### Test Account Funding Strategy

**For Anvil Tests:**
- Use `anvil_setBalance` to fund accounts instantly (no faucet)
- Default balance: 100 ETH per account (sufficient for all tests)

**For Real Testnet Tests (optional):**
- Use Hoodi faucet (manual, not automated)
- Pre-fund accounts before test run
- Monitor balances, alert if below 1 ETH

### Edge Case: Insufficient Balance Test

```rust
#[tokio::test]
async fn pipeline_handles_insufficient_balance() {
    let anvil = start_anvil_fork().await;
    let from_address = anvil.addresses()[0];
    
    // Fund account with only 0.01 ETH
    fund_account(&anvil, from_address, parse_ether("0.01").unwrap()).await;
    
    // Try to send 1 ETH (insufficient balance + gas)
    let txn = build_eth_transfer(
        from_address,
        Address::random(),
        parse_ether("1").unwrap(),
    );
    
    let lobby = spawn_lobby_with_anvil(&anvil).await;
    let execution_id = ExecutionId::new_v4();
    
    lobby.submit(execution_id, txn, test_client_config()).await.unwrap();
    
    let status = poll_until_terminal(&lobby, execution_id, Duration::from_secs(30)).await;
    
    // Should fail at broadcast stage (RPC rejects underpriced tx)
    assert!(matches!(status, PipelineStatus::Failed { stage, .. } if stage == "broadcast"));
}
```

---

## Failure Mode Coverage Matrix

### Priority Failure Modes (from user requirements)

| Failure Mode | Layer | Test Count | Critical? |
|--------------|-------|-----------|-----------|
| **Transaction Reversion** | E2E, Chaos | 10 | ✅ Yes |
| **Actor Panics** | Component, Chaos | 15 | ✅ Yes |
| **Nonce Conflicts** | Integration, Property | 20 | ✅ Yes |
| Database Connection Loss | Chaos | 5 | ⚠️ Medium |
| RPC Timeout/Failure | Component, Chaos | 10 | ⚠️ Medium |
| Lease Expiration Races | Property, Integration | 8 | ⚠️ Medium |
| Semaphore Exhaustion | Integration | 3 | ⚠️ Medium |
| Invalid Gas Estimation | E2E | 5 | ⚠️ Medium |

**Total: ~76 failure-mode tests**

### Detailed Test Coverage

#### 1. Transaction Reversion (10 tests)

**Scenarios:**
- Insufficient balance (ETH transfer)
- Insufficient balance + gas fees
- ERC-20 transfer without approval
- ERC-20 transfer exceeding balance
- Custom contract revert (require statement fails)
- Out of gas (gas limit too low)
- Invalid function selector
- Reentrancy guard triggered
- Access control violation (onlyOwner)
- Contract deployment with invalid bytecode

**Expected Behavior:**
- Validator detects `receipt.status = 0`
- Status → `Failed { stage: "validator", reason: "transaction reverted" }`
- Nonce is released (`state = 'released'`)

#### 2. Actor Panics (15 tests)

**Scenarios:**
- **Nonce Actor:**
  - Panic in `reserve()` handler → 3 retries, then system crash
  - Panic in `resolve()` handler → 3 retries, then system crash
  - Database connection panic → verify retry logic

- **Sign Actor:**
  - Panic during signature generation → verify nonce released
  - Private key not found → verify error propagation

- **Broadcast Actor:**
  - Panic while encoding RLP → verify nonce released
  - Panic during RPC call → verify retry logic

- **Validator Actor:**
  - Panic during receipt polling → verify retry logic
  - Panic on reorg detection → verify error handling

**Expected Behavior:**
- Retry 3 times (with exponential backoff)
- After 3rd failure → system crashes (no silent corruption)
- Nonce is released before crash (if reserved)
- Logs contain panic stacktrace

#### 3. Nonce Conflicts (20 tests)

**Scenarios:**
- **Concurrent Reservations (same address):**
  - 10 concurrent requests → nonces [0..9] assigned, no duplicates
  - 100 concurrent requests → nonces [0..99] assigned, no gaps
  - 1000 concurrent requests (stress test) → monotonicity preserved

- **Concurrent Reservations (different addresses):**
  - 100 requests across 17 addresses → verify sharding works
  - All requests get nonce 0 (independent counters)

- **Lease Expiration:**
  - Reserve nonce, sleep 6 minutes → new request gets same nonce
  - Reserve nonce, sleep 4:59 → new request blocked (lease active)
  - Reserve at T, finalize at T+10min → success (no lease check on finalize)

- **Release and Reuse:**
  - Reserve nonce 5, release it → next request gets nonce 5 (recycled)
  - Reserve nonces [0, 1, 2], release 1 → next request gets nonce 1 (fills gap)

- **Concurrent Finalize:**
  - Two actors try to finalize same nonce → one succeeds, one no-ops

**Expected Behavior:**
- Nonces are always monotonic (no duplicates)
- Gaps are filled by released nonces
- Lease enforcement is strict (5-minute window)
- Partial unique index prevents conflicts

---

## Benchmarking Strategy

### Goals

1. **Establish Baseline Performance**
   - Current p50/p95/p99 latencies (we have no data yet)
   - Maximum sustainable throughput (with 17 pipeline concurrency)

2. **Detect Regressions**
   - Alert if p95 latency increases by >10%
   - Alert if throughput drops by >5%

3. **Identify Bottlenecks**
   - Which actor is slowest? (Nonce? Broadcast? Validator?)
   - Is database the bottleneck or RPC calls?

4. **Guide Optimization**
   - Measure impact of code changes (e.g., "Does async batching help?")

### Benchmark Scenarios

#### 1. Single Pipeline Latency (p50/p95/p99)

**What:** Measure end-to-end latency for a successful transaction (internal stages only, no blockchain)

**Setup:**
- Real Postgres (via testcontainers)
- Mock RPC (wiremock, instant responses)
- Single request per iteration

**Measurement:**
```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn benchmark_single_pipeline(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (db, orchestrator, _mock_rpc) = rt.block_on(setup_benchmark_env());
    
    c.bench_function("pipeline_latency", |b| {
        b.to_async(&rt).iter(|| async {
            let execution_id = ExecutionId::new_v4();
            let txn = build_test_transaction();
            let client_config = build_test_client_config();
            
            let start = std::time::Instant::now();
            orchestrator.submit(execution_id, txn, client_config).await.unwrap();
            wait_for_terminal_status(&orchestrator, execution_id).await;
            start.elapsed()
        });
    });
}

criterion_group!(benches, benchmark_single_pipeline);
criterion_main!(benches);
```

**Target:** < 100ms p95 (internal stages only)

#### 2. Concurrent Pipeline Throughput

**What:** Measure how many transactions/sec Lobby can process under load

**Setup:**
- Real Postgres
- Mock RPC (instant responses)
- 100 concurrent requests submitted simultaneously

**Measurement:**
```rust
fn benchmark_concurrent_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (db, orchestrator, _mock_rpc) = rt.block_on(setup_benchmark_env());
    
    c.bench_function("concurrent_throughput", |b| {
        b.to_async(&rt).iter(|| async {
            let mut handles = vec![];
            
            for _ in 0..100 {
                let orch = orchestrator.clone();
                let exec_id = ExecutionId::new_v4();
                let txn = build_test_transaction();
                
                handles.push(tokio::spawn(async move {
                    orch.submit(exec_id, txn, test_client_config()).await
                }));
            }
            
            // Wait for all to complete
            for handle in handles {
                handle.await.unwrap().unwrap();
            }
        });
    });
}
```

**Target:** > 1000 TPS (with 17 concurrent pipelines)

#### 3. Actor Latency

**What:** Measure isolated actor performance

**Setup (example for nonce actor):**
- Real Postgres
- Single Nonce actor (no other actors)

**Measurement:**
```rust
fn benchmark_nonce_reservation(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (db, nonce_handle) = rt.block_on(setup_nonce_actor());
    
    c.bench_function("nonce_reserve", |b| {
        b.to_async(&rt).iter(|| async {
            let exec_id = ExecutionId::new_v4();
            nonce_handle.reserve(ChainId(1), Address::random(), exec_id).await
        });
    });
}
```

**Target:** < 5ms p95

#### 4. Database Query Performance

**What:** Measure raw query latency (bypassing actors)

**Setup:**
- Real Postgres
- Direct sqlx query execution

**Measurement:**
```rust
fn benchmark_nonce_reserve_query(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let db = rt.block_on(setup_test_db());
    
    c.bench_function("nonce_reserve_query", |b| {
        b.to_async(&rt).iter(|| async {
            let exec_id = ExecutionId::new_v4();
            sqlx::query!(/* TOCTOU-safe reserve query */)
                .fetch_one(&db)
                .await
        });
    });
}
```

**Target:** < 2ms p95

#### 5. Actor Message Passing Overhead

**What:** Measure pure mpsc channel latency (no business logic)

**Setup:**
- In-memory actor (no database)
- Simple echo command

**Measurement:**
```rust
fn benchmark_actor_message_passing(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (tx, mut rx) = mpsc::channel(64);
    
    rt.spawn(async move {
        while let Some((value, reply)) = rx.recv().await {
            let _ = reply.send(value);
        }
    });
    
    c.bench_function("mpsc_round_trip", |b| {
        b.to_async(&rt).iter(|| async {
            let (reply_tx, reply_rx) = oneshot::channel();
            tx.send((42, reply_tx)).await.unwrap();
            reply_rx.await.unwrap()
        });
    });
}
```

**Target:** < 100μs p95

### Benchmark Execution

**When to run:**
- **Locally:** On-demand (via `cargo bench`)
- **CI/CD:** Never (too slow, too resource-intensive)
- **Nightly:** Automated run on dedicated hardware, results stored in artifact

**Regression Detection:**
```toml
# Criterion.toml
[benches]
measurement_time = 30  # 30 seconds per benchmark
sample_size = 100

[comparison]
significance_level = 0.05
noise_threshold = 0.05  # 5% noise tolerance
```

**Output:**
```
pipeline_latency        time:   [85.2 ms 87.4 ms 89.6 ms]
                        change: [-2.3% +0.5% +3.2%] (p = 0.23 > 0.05)
                        No change detected.

concurrent_throughput   time:   [8.2 s 8.5 s 8.8 s]
                        thrpt:  [1176 elem/s 1200 elem/s 1224 elem/s]
                        
nonce_reserve          time:   [3.1 ms 3.3 ms 3.5 ms]
                        change: [+12.3% +15.2% +18.1%] (p = 0.00 < 0.05)
                        ⚠️  Performance regression detected!
```

---

## CI/CD Pipeline Architecture

### GitLab CI Overview

**Goals:**
1. **Automated Testing:** Run all tests on every commit
2. **Gated Merges:** Block PRs if tests fail
3. **Fast Feedback:** < 10 minutes for full test suite
4. **Reproducible Builds:** Docker-based, no "works on my machine"

### Pipeline Stages

```yaml
# .gitlab-ci.yml

stages:
  - lint        # Clippy + rustfmt (< 1 min)
  - unit        # Unit tests (< 1 min)
  - component   # Component tests (< 3 min)
  - integration # Integration tests (< 5 min)
  - e2e         # E2E tests (< 10 min)
  - benchmark   # Optional, nightly only

variables:
  CARGO_HOME: $CI_PROJECT_DIR/.cargo
  RUST_BACKTRACE: 1

# Cache dependencies across jobs
cache:
  key: ${CI_COMMIT_REF_SLUG}
  paths:
    - .cargo/
    - target/

```

### Stage 1: Lint (< 1 minute)

```yaml
lint:clippy:
  stage: lint
  image: rust:1.80
  script:
    - rustup component add clippy
    - cargo clippy --all-targets --all-features -- -D warnings
  rules:
    - if: '$CI_PIPELINE_SOURCE == "merge_request_event"'

lint:format:
  stage: lint
  image: rust:1.80
  script:
    - rustup component add rustfmt
    - cargo fmt --all -- --check
  rules:
    - if: '$CI_PIPELINE_SOURCE == "merge_request_event"'
```

### Stage 2: Unit Tests (< 1 minute)

```yaml
test:unit:
  stage: unit
  image: rust:1.80
  script:
    - cd entropy/unit-tests
    - cargo test --release
  rules:
    - if: '$CI_PIPELINE_SOURCE == "merge_request_event"'
```

### Stage 3: Component Tests (< 3 minutes)

```yaml
test:component:
  stage: component
  image: rust:1.80
  services:
    - postgres:16
    - docker:dind  # Docker-in-Docker for testcontainers
  variables:
    DOCKER_HOST: tcp://docker:2376
    DOCKER_TLS_CERTDIR: "/certs"
    POSTGRES_DB: test_db
    POSTGRES_USER: postgres
    POSTGRES_PASSWORD: postgres
  script:
    - cd entropy/component-tests
    - cargo test --release
  rules:
    - if: '$CI_PIPELINE_SOURCE == "merge_request_event"'
```

### Stage 4: Integration Tests (< 5 minutes)

```yaml
test:integration:
  stage: integration
  image: rust:1.80
  services:
    - postgres:16
    - docker:dind
  variables:
    DOCKER_HOST: tcp://docker:2376
    DOCKER_TLS_CERTDIR: "/certs"
  script:
    - cd entropy/integration-tests
    - cargo test --release -- --test-threads=4  # Parallel execution
  rules:
    - if: '$CI_PIPELINE_SOURCE == "merge_request_event"'
```

### Stage 5: E2E Tests (< 10 minutes)

```yaml
test:e2e:
  stage: e2e
  image: rust:1.80
  services:
    - postgres:16
    - docker:dind
  variables:
    DOCKER_HOST: tcp://docker:2376
    DOCKER_TLS_CERTDIR: "/certs"
  before_script:
    # Start Anvil in background
    - docker run -d --name anvil -p 8545:8545 ghcr.io/foundry-rs/foundry:latest anvil --host 0.0.0.0
  script:
    - cd entropy/e2e-tests
    - export ANVIL_ENDPOINT=http://anvil:8545
    - cargo test --release
  after_script:
    - docker stop anvil
  rules:
    - if: '$CI_PIPELINE_SOURCE == "merge_request_event"'
```

### Stage 6: Benchmarks (Nightly Only)

```yaml
benchmark:performance:
  stage: benchmark
  image: rust:1.80
  services:
    - postgres:16
  script:
    - cd entropy/benchmarks
    - cargo bench --no-fail-fast
    - cp -r target/criterion/ benchmark-results/
  artifacts:
    paths:
      - benchmark-results/
    expire_in: 30 days
  rules:
    - if: '$CI_PIPELINE_SOURCE == "schedule"'  # Nightly cron job
```

### Merge Request Gating

```yaml
# Require all test stages to pass before merge
workflow:
  rules:
    - if: '$CI_PIPELINE_SOURCE == "merge_request_event"'
      when: always
    - if: '$CI_COMMIT_BRANCH == "main"'
      when: always

# Block merge if any test fails
merge_request:
  approval_rules:
    - name: "Tests must pass"
      approvals_required: 0
      when: 'pipeline_succeeds'
```

### Pipeline Visualization

```
Merge Request Created
  │
  ▼
┌─────────────────────┐
│  Lint (1 min)       │  ← Clippy + rustfmt
│  ✓ Pass             │
└─────────────────────┘
  │
  ▼
┌─────────────────────┐
│  Unit (1 min)       │  ← Fast, no I/O
│  ✓ Pass             │
└─────────────────────┘
  │
  ▼
┌─────────────────────┐
│  Component (3 min)  │  ← Real Postgres
│  ✓ Pass             │
└─────────────────────┘
  │
  ▼
┌─────────────────────┐
│  Integration (5 min)│  ← Multi-actor
│  ✓ Pass             │
└─────────────────────┘
  │
  ▼
┌─────────────────────┐
│  E2E (10 min)       │  ← Full system + Anvil
│  ✓ Pass             │
└─────────────────────┘
  │
  ▼
✅ All Checks Passed
   Merge Allowed
```

---

## Directory Structure & Organization

```
entropy/                              # Test workspace root
├── Cargo.toml                        # Workspace manifest
├── README.md                         # Testing philosophy, quick start
│
├── unit-tests/                       # Pure unit tests (no I/O)
│   ├── Cargo.toml
│   └── tests/
│       ├── kernel_types.rs           # ExecutionId, ChainId, etc.
│       ├── error_handling.rs         # Error conversions
│       ├── retry_logic.rs            # Backoff calculations
│       └── utils.rs                  # Helper function tests
│
├── component-tests/                  # Single actor tests
│   ├── Cargo.toml
│   └── tests/
│       ├── nonce_actor.rs            # Reserve, resolve, lease expiration
│       ├── sign_actor.rs             # Signature correctness
│       ├── broadcast_actor.rs        # RPC submission, retries
│       ├── validator_actor.rs        # Receipt polling, confirmation
│       └── relay_host.rs             # Intent persistence, idempotency
│
├── integration-tests/                # Multi-actor orchestration
│   ├── Cargo.toml
│   └── tests/
│       ├── pipeline_happy_path.rs    # Full pipeline success
│       ├── pipeline_failures.rs      # Nonce release on failure
│       ├── concurrent_nonces.rs      # Race condition tests
│       ├── idempotency.rs            # Duplicate execution_id
│       └── status_tracking.rs        # StatusRegistry lifecycle
│
├── property-tests/                   # Invariant verification
│   ├── Cargo.toml
│   └── tests/
│       ├── nonce_monotonicity.rs     # Nonces always increase
│       ├── lease_expiration.rs       # 5-min lease enforcement
│       ├── state_transitions.rs      # Valid status progressions
│       └── idempotency_fuzzing.rs    # Random retry patterns
│
├── chaos-tests/                      # Fault injection
│   ├── Cargo.toml
│   └── tests/
│       ├── database_failures.rs      # Connection loss, pool exhaustion
│       ├── rpc_failures.rs           # Timeouts, rate limits
│       ├── actor_panics.rs           # Panic recovery (3 retries)
│       └── lease_races.rs            # Exactly-5-minute edge cases
│
├── e2e-tests/                        # Full system tests
│   ├── Cargo.toml
│   └── tests/
│       ├── submit_via_http.rs        # API endpoint tests
│       ├── status_polling.rs         # GET /status/{execution_id}
│       ├── eth_transfer.rs           # Simple ETH transfer
│       ├── erc20_transfer.rs         # ERC-20 token transfer
│       ├── uniswap_swap.rs           # Uniswap V2 swap
│       ├── contract_deployment.rs    # Deploy contract
│       └── transaction_reversion.rs  # On-chain revert handling
│
├── benchmarks/                       # Performance profiling
│   ├── Cargo.toml
│   └── benches/
│       ├── pipeline_latency.rs       # p50/p95/p99 latencies
│       ├── concurrent_throughput.rs  # TPS under load
│       ├── nonce_reservation.rs      # Isolated actor performance
│       ├── db_queries.rs             # Raw query benchmarks
│       └── message_passing.rs        # mpsc overhead
│
└── test-utils/                       # Shared test helpers
    ├── Cargo.toml
    └── src/
        ├── lib.rs
        ├── fixtures.rs               # Test transactions, API keys, accounts
        ├── db_helpers.rs             # Testcontainers setup, migrations
        ├── anvil_helpers.rs          # Anvil fork, account funding
        ├── mock_rpc.rs               # Wiremock RPC server
        ├── assertions.rs             # Custom assert macros
        └── wait_utils.rs             # Event-driven wait helpers
```

### Workspace `Cargo.toml`

```toml
[workspace]
members = [
    "unit-tests",
    "component-tests",
    "integration-tests",
    "property-tests",
    "chaos-tests",
    "e2e-tests",
    "benchmarks",
    "test-utils",
]

[workspace.dependencies]
# Core Lobby dependencies (imported from ../ensemble)
lobby-kernel = { path = "../ensemble/crate/kernel" }
lobby-actors = { path = "../ensemble/crate/actors" }
lobby-cortex = { path = "../ensemble/crate/cortex" }
lobby-utils = { path = "../ensemble/crate/utils" }

# Testing frameworks
tokio = { version = "1.40", features = ["full", "test-util"] }
tokio-test = "0.4"
rstest = "0.23"
mockall = "0.13"
wiremock = "0.6"
proptest = "1.5"
quickcheck = "1.0"

# Database testing
sqlx = { version = "0.8", features = ["postgres", "runtime-tokio-rustls", "macros"] }
testcontainers = "0.23"
testcontainers-modules = { version = "0.11", features = ["postgres"] }

# Blockchain testing
alloy = { version = "0.8", features = ["full"] }
foundry-config = "0.2"

# Benchmarking
criterion = { version = "0.5", features = ["html_reports", "async_tokio"] }

# Utilities
serde_json = "1.0"
uuid = { version = "1.11", features = ["v4"] }
hex = "0.4"
rand = "0.8"
```

---

## Running Tests Locally

### Prerequisites

```bash
# Install Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install Docker (for testcontainers and Anvil)
# Follow official Docker installation guide for your OS

# Install SQLx CLI
cargo install sqlx-cli --no-default-features --features postgres

# Install Foundry (for Anvil)
curl -L https://foundry.paradigm.xyz | bash
foundryup
```

### Running All Tests

```bash
cd entropy

# Run full test suite (unit + component + integration + e2e)
cargo test --workspace --release

# Run with verbose output
cargo test --workspace --release -- --nocapture

# Run specific test layer
cargo test -p component-tests --release
cargo test -p e2e-tests --release
```

### Running Individual Tests

```bash
# Run single test file
cargo test -p integration-tests --test pipeline_happy_path

# Run single test function
cargo test -p component-tests --test nonce_actor nonce_actor_prevents_duplicate_reservation

# Run with logging
RUST_LOG=debug cargo test -p e2e-tests --test eth_transfer -- --nocapture
```

### Running Benchmarks

```bash
cd entropy/benchmarks

# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench --bench pipeline_latency

# Generate flamegraph (requires cargo-flamegraph)
cargo install flamegraph
cargo flamegraph --bench pipeline_latency
```

### Environment Variables

```bash
# Enable debug logging
export RUST_LOG=debug

# Use specific Postgres (if not using testcontainers)
export DATABASE_URL=postgres://postgres:password@localhost/lobby_test

# Use specific Anvil endpoint
export ANVIL_ENDPOINT=http://localhost:8545

# Override retry config for faster tests
export RETRY_MAX_ATTEMPTS=1
export RETRY_BASE_DELAY_MS=10
```

---

## Writing New Tests

### Test Naming Conventions

**Pattern:** `<component>_<behavior>_<expected_outcome>`

**Examples:**
- ✅ `nonce_actor_prevents_duplicate_reservation`
- ✅ `pipeline_releases_nonce_on_broadcast_failure`
- ✅ `validator_detects_transaction_reversion`
- ❌ `test1` (too vague)
- ❌ `nonce_test` (not descriptive)

### Test Structure Template

```rust
#[tokio::test]
async fn <test_name>() {
    // 1. ARRANGE: Set up test environment
    let (db, orchestrator, mock_rpc) = setup_test_environment().await;
    let test_data = build_test_transaction();
    
    // 2. ACT: Execute the behavior being tested
    let result = orchestrator.submit(execution_id, test_data, client_config).await;
    
    // 3. ASSERT: Verify expected outcomes
    assert!(result.is_ok());
    
    // 4. VERIFY: Check database state
    let db_state = query_database_state(&db, execution_id).await;
    assert_eq!(db_state.nonce_state, "finalized");
    
    // 5. CLEANUP: Explicit cleanup if needed (usually handled by Drop)
}
```

### Common Assertions

```rust
// Status assertions
assert_eq!(status, PipelineStatus::Confirmed { tx_hash });
assert!(matches!(status, PipelineStatus::Failed { .. }));

// Database state assertions
assert_nonce_state(&db, execution_id, "released").await;
assert_row_count(&db, "nonce.nonce_assignments", 1).await;

// Timing assertions (avoid sleep, use event-driven waits)
let status = wait_for_terminal_status(&orchestrator, execution_id, Duration::from_secs(10)).await;

// Error assertions
assert!(result.is_err());
assert!(matches!(result, Err(LocalError::AlreadyReserved)));
```

### Writing Property Tests

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn property_name(
        input1 in strategy1(),
        input2 in strategy2(),
    ) {
        // Arrange
        let rt = tokio::runtime::Runtime::new().unwrap();
        
        // Act + Assert (inside async block)
        rt.block_on(async {
            let result = test_operation(input1, input2).await;
            prop_assert!(invariant_holds(result));
        });
    }
}

// Strategies for common types
fn execution_id_strategy() -> impl Strategy<Value = ExecutionId> {
    any::<[u8; 16]>().prop_map(|bytes| ExecutionId::from_bytes(&bytes).unwrap())
}

fn address_strategy() -> impl Strategy<Value = Address> {
    any::<[u8; 20]>().prop_map(Address::from)
}
```

---

## Debugging Failed Tests

### Common Failure Patterns

#### 1. **Testcontainer Connection Refused**

**Symptom:**
```
Connection refused (os error 111) at postgres://localhost:5432
```

**Cause:** Docker not running or testcontainers failed to start

**Fix:**
```bash
# Ensure Docker daemon is running
sudo systemctl start docker

# Check Docker status
docker ps

# Manually test Postgres container
docker run -d -p 5432:5432 postgres:16
```

#### 2. **Test Timeout (Hanging)**

**Symptom:** Test runs forever, no output

**Cause:** Deadlock in actor message passing or missing `await`

**Debug:**
```rust
// Add timeout to test
#[tokio::test(flavor = "multi_thread")]
async fn test_with_timeout() {
    tokio::time::timeout(Duration::from_secs(5), async {
        // Test code
    })
    .await
    .expect("Test timed out");
}

// Enable tracing to see where it hangs
RUST_LOG=trace cargo test <test_name> -- --nocapture
```

#### 3. **Flaky Test (Passes Sometimes)**

**Symptom:** Test passes locally but fails in CI (or vice versa)

**Cause:** Race condition, timing dependency, or resource contention

**Fix:**
```rust
// Replace sleep with event-driven wait
// ❌ Bad: tokio::time::sleep(Duration::from_secs(1)).await;

// ✅ Good: Wait for specific condition
async fn wait_for_status(
    registry: &StatusRegistry,
    execution_id: ExecutionId,
    expected: PipelineStatus,
    timeout: Duration,
) -> PipelineStatus {
    let deadline = Instant::now() + timeout;
    
    loop {
        if let Some(status) = registry.get(execution_id) {
            if status == expected {
                return status;
            }
        }
        
        if Instant::now() > deadline {
            panic!("Timeout waiting for status {:?}", expected);
        }
        
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
```

#### 4. **Database Migration Mismatch**

**Symptom:**
```
Table 'nonce.nonce_assignments' does not exist
```

**Cause:** Migrations not applied or wrong migration path

**Fix:**
```rust
// Ensure correct migration path
sqlx::migrate!("../../ensemble/database/migrations")
    .run(&db)
    .await
    .expect("Failed to apply migrations");

// Verify migration path exists
// Should see: ensemble/database/migrations/*.sql
```

---

## Performance Regression Detection

### Criterion Configuration

```toml
# entropy/benchmarks/Criterion.toml
[default]
measurement_time = 30           # 30 seconds per benchmark
sample_size = 100               # 100 samples
warm_up_time = 5                # 5 seconds warm-up
significance_level = 0.05       # 5% significance threshold
noise_threshold = 0.05          # 5% noise tolerance
confidence_level = 0.95

[benches.pipeline_latency]
measurement_time = 60           # Longer for critical path
sample_size = 200

[benches.concurrent_throughput]
measurement_time = 120
sample_size = 50
```

### Regression Alerts

**Criterion automatically detects regressions:**

```
pipeline_latency        time:   [95.2 ms 98.4 ms 101.6 ms]
                        change: [+15.3% +18.2% +21.1%] (p = 0.00 < 0.05)
                        ⚠️  Performance regression detected!
```

**Action on Regression:**
1. Run `git bisect` to find offending commit
2. Profile with `cargo flamegraph --bench <benchmark_name>`
3. Identify hot path (database query? RPC call? message passing?)
4. Optimize or revert change

### Flamegraph Profiling

```bash
# Install flamegraph tool
cargo install flamegraph

# Profile specific benchmark
cargo flamegraph --bench pipeline_latency

# Open generated flamegraph.svg in browser
firefox flamegraph.svg
```

**Interpreting Flamegraphs:**
- **Wide bars:** Function consumes significant CPU time
- **Deep stacks:** Function has many nested calls (potential optimization target)
- **Common hot paths:**
  - `sqlx::query` → Database query optimization
  - `alloy::provider::send_request` → RPC call optimization
  - `tokio::sync::mpsc::send` → Message passing overhead

---

## Future Enhancements

### Phase 1: Current Focus (0.1.0)
- ✅ Core test infrastructure (unit, component, integration, E2E)
- ✅ Property-based testing (nonce monotonicity, idempotency)
- ✅ Chaos testing (database failures, RPC timeouts, actor panics)
- ✅ GitLab CI pipeline (gated PR merges)
- ✅ Benchmarking suite (Criterion, baseline establishment)

### Phase 2: Observability (0.2.0)
- 📍 Distributed tracing (OpenTelemetry integration)
- 📍 Metrics collection (Prometheus exporter)
- 📍 Test coverage reports (tarpaulin)
- 📍 Mutation testing (cargo-mutants)

### Phase 3: Advanced Testing (0.3.0)
- 📍 Load testing (k6 or Gatling for HTTP stress tests)
- 📍 Chaos engineering (controlled actor kills, network partitions)
- 📍 Fuzz testing (cargo-fuzz for input validation)
- 📍 Snapshot testing (insta for database state snapshots)

### Phase 4: Production Readiness (0.4.0)
- 📍 Real testnet E2E tests (Hoodi with real RPC, nightly runs)
- 📍 Soak testing (24-hour continuous load)
- 📍 Backward compatibility tests (migration from v0.1 → v0.2)
- 📍 Performance profiling in production (eBPF tracing)

---

## Appendix: Key Terminology

**Term** | **Definition**
---|---
**TOCTOU** | Time-of-check / time-of-use race condition (prevented by atomic queries)
**Lease** | Time-bound reservation (5 minutes in Lobby) to prevent duplicate processing
**Revision** | Incremental counter for audit trails (each state change = new revision)
**Shard** | One instance of a sharded actor (e.g., Nonce Actor #3 of 17)
**Pipeline** | Sequential execution of all 5 actor stages for one transaction
**Semaphore** | Concurrency primitive limiting simultaneous pipelines (default: 17)
**Anvil** | Local Ethereum node (part of Foundry) for deterministic blockchain testing
**Testcontainers** | Rust library for running Docker containers in tests
**Wiremock** | HTTP mocking library for simulating RPC providers
**Proptest** | Property-based testing library (fuzzing with invariant verification)
**Criterion** | Statistical benchmarking framework for Rust

---

**End of Testing Strategy Document**

For architecture details, see [Lobby_Dev_Doc.md](../../ensemble/docs/Lobby_Dev_Doc.md).  
For API specifications, see [Client_API_Doc.md](../../ensemble/docs/Client_API_Doc.md).

---

*This document is a living guide. Update it as testing strategies evolve.*
