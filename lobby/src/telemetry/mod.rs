//! This module provides a lock-free registry for actors to update,
//! and a background exporter that streams deltas to a Unix Domain Socket (UDS).

use dashmap::DashMap;
use primitives::types::ExecutionId;
use quanta::Clock;
use std::sync::Arc;
use tokio::sync::mpsc;

// ===========================================================
// primary struct and types

/// Nanosecond-precision timestamps for pipeline stages.
#[derive(Debug, Clone)]
pub struct PipelineTelemetry {
    pub relayhost_ns: u64,
    pub nonce_ns: u64,
    pub sign: u64,
    pub broadcast: u64,
}

/// Lock-free registry mapping ExecutionId to its telemetry state.
pub type TelemetryRegistry = Arc<DashMap<ExecutionId, PipelineTelemetry>>;

/// Events sent from actors to the background exporter.
#[derive(Debug, Clone)]
pub enum TelemetryEvent {
    StateComplete {
        execution_id: ExecutionId,
        stage: &'static str,
        timestamp_ns: u64,
    },
    PipelineComplete {
        execution_id: ExecutionId,
    },
}

/// Lobby `telemetry-context` holder.
#[derive(Clone)]
pub struct TelemetryContext {
    registry: TelemetryRegistry,
    clock: Clock,
    tx: mpsc::UnboundedSender<TelemetryEvent>,
}

impl TelemetryContext {
    pub fn new(tx: mpsc::UnboundedSender<TelemetryEvent>) -> Self {
        TelemetryContext {
            registry: Arc::new(DashMap::new()),
            clock: Clock::new(),
            tx,
        }
    }
}
