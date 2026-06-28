use dashmap::DashMap;
use primitives::types::ExecutionId;
use quanta::Instant;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::mpsc;

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
    PipelineComplete { execution_id: ExecutionId },
}

/// Lobby `telemetry-context` holder.
pub struct TelemetryContext {
    registry: TelemetryRegistry,
    tx: mpsc::UnboundedSender<TelemetryEvent>,
}

impl Clone for TelemetryContext {
    fn clone(&self) -> Self {
        let tx2 = self.tx.clone();
        Self {
            registry: Arc::clone(&self.registry),
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
            tx,
        }
    }

    /// record the starting instant, to use as a reference.
    #[inline(always)]
    pub fn record_start(&self, execution_id: ExecutionId) {
        let now = Instant::now();
        let mut entry = self
            .registry
            .entry(execution_id)
            .or_insert_with(PipelineTelemetry::default);
        entry.start = Some(now);
    }

    /// Called by pipeline, upon successful stage completion. O(1) lock-free operation, non-blocking.
    #[inline(always)]
    pub fn record_stage(&self, stage: TelemetryStage, execution_id: ExecutionId) {
        let now = Instant::now();
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
