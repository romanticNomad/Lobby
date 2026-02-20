//! # Orchestrator
//!
//! Wires together all Lobby actors into a single pipeline:
//!
//! ```text
//! submit()
//!   └─ RelayHost.send_transaction()   [retried, idempotent]
//!        └─ spawn pipeline task (semaphore-gated)
//!              ├─ Nonce.reserve()     [retried]
//!              ├─ Sign.sign()         [retried; releases nonce on hard-fail]
//!              ├─ Broadcast.broadcast()[retried; releases nonce on hard-fail]
//!              └─ Validator.validate()[retried; releases nonce on hard-fail]
//! ```
//!
//! The `OrchestratorHandle` is a cheap `Arc`-backed clone that can be placed
//! in Axum's `AppState`.

pub mod config;
pub mod error;
pub mod pipeline;
pub mod pool;
pub mod retry;
pub mod state;

// ============================================================
