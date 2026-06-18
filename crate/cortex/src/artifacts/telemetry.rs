//! This module provides a lock-free registry for actors to update stage timestamps,
//! and a background exporter that streams deltas to a Unix Domain Socket (UDS).

use dashmap::DashMap;
use futures_util::sink::SinkExt;
use primitives::types::ExecutionId;
use quanta::{Clock, Instant};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::codec::{FramedWrite, LinesCodec};

// ===========================================================
// primary struct and types

/// Enum to identify pipeline stage
#[derive(Debug, Clone)]
pub enum TelemetryStage {
    RelayHost,
    Nonce,
    Sign,
    Broadcast,
}

/// Nanosecond-precision timestamps for pipeline stages.
#[derive(Debug, Clone, Default)]
pub struct PipelineTelemetry {
    pub start: Option<Instant>,
    pub relayhost: Option<Instant>,
    pub nonce: Option<Instant>,
    pub sign: Option<Instant>,
    pub broadcast: Option<Instant>,
}

/// Lock-free registry mapping ExecutionId to its telemetry state.
pub type TelemetryRegistry = Arc<DashMap<ExecutionId, PipelineTelemetry>>;

/// Events sent from actors to the background exporter.
#[derive(Debug, Clone)]
pub enum TelemetryEvent {
    StateComplete {
        execution_id: ExecutionId,
        stage: TelemetryStage,
        timestamp: Option<Instant>,
    },
    PipelineComplete {
        execution_id: ExecutionId,
    },
}

/// Lobby `telemetry-context` holder.
pub struct TelemetryContext {
    registry: TelemetryRegistry,
    clock: Clock,
    tx: mpsc::UnboundedSender<TelemetryEvent>,
}

impl Clone for TelemetryContext {
    fn clone(&self) -> Self {
        let tx2 = self.tx.clone();
        Self {
            registry: Arc::clone(&self.registry),
            clock: self.clock.clone(),
            tx: tx2,
        }
    }
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
        let now = self.clock.recent();
        let mut entry = self
            .registry
            .entry(execution_id)
            .or_insert_with(PipelineTelemetry::default);
        entry.start = Some(now);
    }

    /// Called by pipeline, upon successful stage completion. O(1) lock-free operation, non-blocking.
    #[inline(always)]
    pub fn record_stage(&self, stage: TelemetryStage, execution_id: ExecutionId) {
        let now = self.clock.recent();
        let mut entry = self
            .registry
            .entry(execution_id)
            .or_insert_with(PipelineTelemetry::default);

        match stage {
            TelemetryStage::RelayHost => entry.relayhost = Some(now),
            TelemetryStage::Nonce => entry.nonce = Some(now),
            TelemetryStage::Sign => entry.sign = Some(now),
            TelemetryStage::Broadcast => entry.broadcast = Some(now),
        }

        let _ = self.tx.send(TelemetryEvent::StateComplete {
            execution_id,
            stage,
            timestamp: Some(now),
        });
    }

    /// Called at the end of the pipeline (success or terminal failure) to prevent memory leaks.
    #[inline(always)]
    pub fn finalize(&self, execution_id: ExecutionId) {
        let _ = self
            .tx
            .send(TelemetryEvent::PipelineComplete { execution_id });
    }

    /// get a registry clone for realtime metric export.
    #[inline(always)]
    pub fn get_registry(&self) -> TelemetryRegistry {
        Arc::clone(&self.registry)
    }
}

// ===========================================================
// reponse time calculation and UDS streaming

/// Background task that calculates deltas and streams NDJSON to the benchmark harness via UDS.
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
                let record = LatencyRecord {
                    execution_id: execution_id.0.to_string(),
                    relayhost_duration_us: get_duration_us(telemetry.start, telemetry.relayhost),
                    nonce_duration_us: get_duration_us(telemetry.relayhost, telemetry.nonce),
                    sign_duration_us: get_duration_us(telemetry.nonce, telemetry.sign),
                    broadcast_duration_us: get_duration_us(telemetry.sign, telemetry.broadcast),
                    total_pipeline_us: get_duration_us(telemetry.start, telemetry.broadcast),
                };

                match serde_json::to_string(&record) {
                    Ok(json_payload) => {
                        if let Err(err) = framed.send(json_payload).await {
                            tracing::error!(%execution_id, %err, "Failed to send telemetry data to socket.");
                            break;
                        }
                    }

                    Err(err) => {
                        tracing::error!(%execution_id, %err, "Failed to serialize telemetry data.");
                    }
                }
            }
        }
    }
}

// ============================================================
// helper function

/// Calculate `Duration` betweeen 2 subsequent stages in `micro_second`
pub fn get_duration_us(stage_a: Option<Instant>, stage_b: Option<Instant>) -> u64 {
    match (stage_a, stage_b) {
        (Some(a), Some(b)) => b.duration_since(a).as_micros() as u64,
        _ => 0,
    }
}

// ============================================================
