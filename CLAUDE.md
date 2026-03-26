# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Lobby is a high-performance blockchain transaction signing service built in Rust. It uses an actor-based architecture with Tokio for async runtime, Axum for HTTP, PostgreSQL for persistence, and Redis for status tracking.

**Prototype Notice**: This is a testnet-only project. Do NOT use for mainnet transactions.

## Build and Run Commands

```bash
# Start infrastructure (PostgreSQL + Redis)
cd database && docker compose up -d && cd ..

# Run database migrations
sqlx migrate run

# Run the main server (release mode)
cargo run --release --bin lobby

# Run the main server (debug mode)
cargo run --bin lobby

# Generate API keys from test_keys.json
cargo run --release --bin generate_api_keys

# Build all crates
cargo build --release
```

## Environment Setup

Required environment variables (set in `.env` or export):
- `DATABASE_URL=postgresql://lobby:lobby_dev_password@localhost:5432/lobby-db`
- `REDIS_URL=redis://localhost:6379`
- `SERVER_ADDR=0.0.0.0:3000`
- `LOBBY_API_KEY_<N>=<api_token>:<client_id>:<from_address>` (at least one)
- `RPC_ENDPOINT_<chain_id>=<url>` (e.g., `RPC_ENDPOINT_1=https://...`)

Create `test_keys.json` in project root for local signing keys (prototype only).

## Code Architecture

### Workspace Structure

```
crate/
├── actors/      # Five pipeline actors (relayhost, nonce, sign, broadcast, validator)
├── cortex/      # Orchestrator that coordinates actors and runs pipelines
├── primitives/  # Shared types (ExecutionId, ChainId, TxNonce, etc.) and traits
└── utils/       # EIP-1559 signing, custody (key management), registry (API/RPC config)

lobby/
├── src/main.rs           # Entry point, spawns actors and HTTP server
├── src/server/           # Axum handlers (auth.rs, handler.rs)
├── src/bots/             # Background tasks (sweeper, scanner)
└── src/bin/              # Binaries (generate_api_keys.rs)
```

### Actor Pattern

Each actor follows a three-file structure:
- `engine.rs` - Event loop owning database pool and mpsc::Receiver
- `handle.rs` - Public interface implementing the actor trait
- `mod.rs` - Spawn function creating the actor

### Five-Stage Pipeline

Transactions flow through: RelayHost → Nonce → Sign → Broadcast → Validator

Each stage has dedicated database schemas:
- `relay_host.transaction_intents` - Write-once audit log
- `nonce.nonce_assignments` - Nonce tracking with revision-based state
- `sign.sign_requests` - Signing records
- `broadcast.broadcast_requests` - Broadcast records
- `validator.validation_requests` - Validation records

### Key Traits (primitives/src/traits.rs)

- `IntentRelay` - Persist transaction intent
- `NonceManager` - Reserve/resolve/sync nonces
- `Signer` - Sign transactions
- `Broadcaster` - Submit to RPC
- `Validator` - Poll for inclusion
- `StateStore` - Status registry interface

### Sharding Strategy

- Nonce actors: Sharded by `from_address` (same address → same actor)
- Sign actors: Sharded by `execution_id` (load balancing)
- Broadcast actors: Sharded by `chain_id` (same chain → same actor)
- RelayHost and Validator: Single instance (no sharding)

### Concurrency Control

- `PIPELINE_CONCURRENCY` (default: 17) - Semaphore limiting concurrent pipelines
- Actor buffer size (default: 64) - mpsc channel capacity per actor

## Database Migrations

Migrations are in `database/migrations/` and run automatically at startup via `sqlx::migrate!`.

Each migration creates a schema with tables for that actor's state tracking.

## Background Bots

- **Sweeper Bot** (`bots/sweeper.rs`): Runs every 30s, releases stale `'reserved'` nonces older than 2m5s
- **Scanner Bot** (`bots/scanner.rs`): Runs every 30s, polls for timed-out transactions to resolve their status

## Supported Chains

Hardcoded in `crate/utils/src/eip1559/lint.rs`:
- Ethereum Mainnet (1)
- Hoodi Testnet (560048)
- Polygon (137)
- Arbitrum One (42161)

## Key Configuration (environment variables)

| Variable | Default | Description |
|----------|---------|-------------|
| `NONCE_SHARDS` | 17 | Nonce actor instances |
| `SIGN_SHARDS` | 17 | Sign actor instances |
| `BROADCAST_SHARDS` | 17 | Broadcast actor instances |
| `ACTOR_BUFFER_SIZE` | 64 | mpsc channel buffer |
| `PIPELINE_SEMAPHORE_TIMEOUT_MS` | 5000 | Semaphore wait timeout |

## Running Tests

No dedicated test suite exists. The `test_keys.json` file contains test accounts for local development.

## Important Files

- `docs/Lobby_API_Doc.md` - API documentation for clients
- `docs/Lobby_Bootup_Doc.md` - Quick start guide
- `database/docker-compose.yml` - PostgreSQL + Redis setup
- `.env` - Environment configuration (gitignored)