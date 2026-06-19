use crate::metrics::collector::LatencyRecord;
use anyhow::Context;
use futures_util::StreamExt;
use std::time::{Duration, Instant};
use tokio::net::UnixStream;
use tokio_util::codec::{FramedRead, LinesCodec};
use tracing::{error, info, warn};

mod collector;
pub use collector::MetricsCollector;

// ===========================================================
// UDS Telemetry Stream Consumer

/// Connects to Lobby's telemetry UDS, consumes the NDJSON stream,
/// and aggregates metrics until the stream closes or shutdown is triggered.
pub async fn telemetry_stream_reader(
    socket_path: &str,
    test_start: Instant,
    warmup: Duration,
    steady_state: Duration,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<MetricsCollector> {
    info!("Connecting to Lobby telemetry UDS at: {}", socket_path);

    // Retry logic for UDS connection in case Lobby is still binding the socket
    let stream = tokio::time::timeout(Duration::from_secs(5), UnixStream::connect(socket_path))
        .await
        .context("Waiting for Lobby UDS socket")?
        .context("UDS socket connection timed_out: 5sec")?;

    let mut framed_payload = FramedRead::new(stream, LinesCodec::new());
    let mut collector = MetricsCollector::new(test_start, warmup, steady_state);

    info!("Telemetry collector online. Waiting for data...");
    loop {
        tokio::select! {
            biased;

            // 1. Check for shutdown signal from main orchestrator
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    info!("Shutdown signal received. Finalizing metrics collection.");
                    break;
                }
            }

            // 2. Read next NDJSON line from UDS
            frame = framed_payload.next() => {
                match frame {
                    Some(Ok(line)) => {
                        match serde_json::from_str::<LatencyRecord>(&line) {
                            Ok(latency_record) => {
                                collector.submit_metrics(latency_record, Instant::now());
                            }
                            Err(e) => {
                                warn!("Failed to deserialize telemetry record: {}. Line: {}", e, line);
                            }
                        }
                    }

                    Some(Err(e)) => {
                        error!("UDS stream error: {}", e);
                        break;
                    }

                    None => {
                        info!("Lobby telemetry UDS stream closed by server.");
                        break;
                    }
                }
            }
        }
    }

    anyhow::Ok(collector)
}

// ===========================================================
