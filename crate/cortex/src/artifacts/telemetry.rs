//! This module provides a lock-free registry for actors to update stage timestamps,
//! and a background exporter that streams deltas to a Unix Domain Socket (UDS).

use dashmap::DashMap;
use primitives::types::ExecutionId;
use quanta::Clock;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::codec::{FramedWrite, LinesCodec};

// ===========================================================
// primary struct and types

/// Nanosecond-precision timestamps for pipeline stages.
#[derive(Debug, Clone, Default)]
pub struct PipelineTelemetry {
    pub start_ns: u64,
    pub relayhost_ns: u64,
    pub nonce_ns: u64,
    pub sign_ns: u64,
    pub broadcast_ns: u64,
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

/// The NDJSON payload streamed to the benchmark harness via UDS.
#[derive(Serialize)]
pub struct LatencyRecord {
    pub execution_id: String,
    pub relayhost_duration_us: u64,
    pub nonce_duration_us: u64,
    pub sign_duration_us: u64,
    pub broadcast_duration_us: u64,
    pub total_pipeline_us: u64,
}

// ===========================================================
// method implimentations on TelemetryContext

impl TelemetryContext {
    pub fn new(tx: mpsc::UnboundedSender<TelemetryEvent>) -> Self {
        TelemetryContext {
            registry: Arc::new(DashMap::new()),
            clock: Clock::new(),
            tx,
        }
    }

    /// record the starting instant, to use as a reference.
    #[inline(always)]
    pub fn record_start(&self, execution_id: ExecutionId) {
        let now_ns = self.clock.now_ns() as u64;
        let mut entry = self
            .registry
            .entry(execution_id)
            .or_insert_with(PipelineTelemetry::default);

        entry.start_ns = now_ns;
    }

    /// Called by actors upon successful stage completion. O(1) lock-free operation, non-blocking.
    #[inline(always)]
    pub fn stage_update(&self, stage: &'static str, execution_id: ExecutionId) {
        let now_ns = self.clock.now() as u64;
        let mut entry = self
            .registry
            .entry(execution_id)
            .or_insert_with(PipelineTelemetry::default);

        match stage {
            "relayhost" => entry.relayhost_ns = now_ns,
            "nonce" => entry.nonce_ns = now_ns,
            "sign" => entry.sign_ns = now_ns,
            "broadcast" => entry.broadcast_ns = now_ns,
            _ => {}
        }

        let _ = self.tx.send(TelemetryEvent::StateComplete {
            execution_id,
            stage,
            timestamp_ns: now_ns,
        });
    }

    /// Called at the end of the pipeline (success or terminal failure) to prevent memory leaks.
    #[inline(always)]
    pub fn finalize(&self, execution_id: ExecutionId) {
        let _ = self
            .tx
            .send(TelemetryEvent::PipelineComplete { execution_id });
        self.registry.remove(&execution_id);
    }

    /// get a registry clone for realtime metric export.
    #[inline(always)]
    pub fn get_registry(&self) -> TelemetryRegistry {
        Arc::clone(&self.registry)
    }
}

// ===========================================================
// reponse time calculation and UDS streaming

/// Background task that calculates deltas and
/// streams NDJSON to the benchmark harness via UDS.
pub async fn run_telemetry_exporter(
    mut rx: mpsc::UnboundedReceiver<TelemetryEvent>,
    socket_path: &str,
    registry: TelemetryRegistry,
) {
    // clean socket
    let _ = std::fs::remove_file(socket_path);

    let listener = match tokio::net::UnixListener::bind(socket_path) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(%e, "Failed to bind telemetry UDS.");
            return;
        }
    };

    let (socket, _) = match listener.accept().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(%e, "Failed to accept UDS connection from benchmark harness.");
            return;
        }
    };

    let mut framed = FramedWrite::new(socket, LinesCodec::new());
    while let Some(event) = rx.recv().await {
        if let TelemetryEvent::PipelineComplete { execution_id } = event {
            if let Some((_, telemetry)) = registry.remove(&execution_id) {
                // calculate delats with overflow handling
                let relayhost_duration =
                    (telemetry.relayhost_ns.saturating_sub(telemetry.start_ns)) / 1_000;
                let nonce_duration =
                    (telemetry.nonce_ns.saturating_sub(telemetry.relayhost_ns)) / 1_000;
                let sign_duration = (telemetry.sign_ns.saturating_sub(telemetry.nonce_ns)) / 1_000;
                let broadcast_duration =
                    (telemetry.broadcast_ns.saturating_sub(telemetry.sign_ns)) / 1_000;
                let total_pipeline =
                    (telemetry.broadcast_ns.saturating_sub(telemetry.start_ns)) / 1_000;

                let record = LatencyRecord {
                    execution_id: execution_id.0.to_string(),
                    relayhost_duration_us: relayhost_duration,
                    nonce_duration_us: nonce_duration,
                    sign_duration_us: sign_duration,
                    broadcast_duration_us: broadcast_duration,
                    total_pipeline_us: total_pipeline,
                };

                if let Ok(json_payload) = serde_json::to_string(&record) {
                    let _ = framed.send(json_payload).await;
                }
            }
        }
    }
}

// ============================================================
