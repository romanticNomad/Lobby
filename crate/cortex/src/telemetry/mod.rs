//! This module provides a lock-free registry for actors to update stage timestamps,
//! and a background exporter that streams deltas to a Unix Domain Socket (UDS).

pub mod serialize;

use futures_util::SinkExt;
use quanta::Instant;
pub use serialize::*;
use tokio::sync::mpsc;
use tokio_util::codec::{FramedWrite, LinesCodec};

// ===========================================================
// UDS telemetry stream producer

/// Background task that calculates deltas and streams NDJSON to the benchmark harness via UDS.
pub async fn run_telemetry_exporter(
    mut rx: mpsc::UnboundedReceiver<TelemetryExport>,
    socket_path: &str,
    registry: TelemetryRegistry,
) {
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
        let execution_id = event.execution_id;
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
