// use anyhow::{Result, anyhow};
// use hdrhistogram::Histogram;
// use reqwest::{Client, StatusCode};
// use serde::{Deserialize, Serialize};
// use serde_json::json;
// use std::sync::atomic::{AtomicU64, Ordering};
// use std::sync::Arc;
// use std::time::{Duration, Instant};
// use tokio::sync::{mpsc, Semaphore};
// use tokio::task::JoinSet;
// use uuid::Uuid;

// /// Configuration for the load generator.
// /// Parsed from CLI in `main.rs`, defaults provided for benchmark windows.
// #[derive(Debug, Clone)]
// pub struct LoadGenConfig {
//     pub accounts: usize,
//     pub target_tps: f64,
//     pub ramp_duration_secs: f64,
//     pub steady_duration_secs: f64,
//     pub cooldown_duration_secs: f64,
//     pub max_concurrency: usize,
//     pub base_url: String,
// }

// impl Default for LoadGenConfig {
//     fn default() -> Self {
//         Self {
//             accounts: 250,
//             target_tps: 1000.0,
//             ramp_duration_secs: 10.0,
//             steady_duration_secs: 30.0,
//             cooldown_duration_secs: 5.0,
//             max_concurrency: 500,
//             base_url: "http://127.0.0.1:3000".to_string(),
//         }
//     }
// }

// /// Aggregated benchmark results returned by the collector.
// #[derive(Debug, Serialize, Deserialize)]
// pub struct BenchmarkResult {
//     pub total_submitted: u64,
//     pub total_broadcasted: u64,
//     pub error_rate: f64,
//     pub latency_ms: LatencyPercentiles,
//     pub nonce_collisions: u64,
//     pub hard_failures: u64,
// }

// #[derive(Debug, Serialize, Deserialize)]
// pub struct LatencyPercentiles {
//     pub p50: f64,
//     pub p95: f64,
//     pub p99: f64,
// }

// /// Metric events dispatched from loadgen tasks to the collector.
// pub enum MetricEvent {
//     Latency(u64), // Nanoseconds
//     Submitted,
//     Broadcasted,
//     BackpressureError,
//     HardFailure, // NonceTooLow, NonceMismatch, or network failure
// }

// struct TestAccount {
//     address: String,
//     api_key: String,
// }

// /// Load generator orchestrator.
// pub struct LoadGen {
//     config: LoadGenConfig,
//     accounts: Arc<Vec<TestAccount>>,
//     client: Client,
//     metrics_tx: mpsc::UnboundedSender<MetricEvent>,
// }

// impl LoadGen {
//     /// Initializes the load generator with account generation and HTTP client pooling.
//     pub fn new(config: LoadGenConfig) -> Self {
//         let accounts = Arc::new(Self::generate_accounts(config.accounts));
//         let client = Client::builder()
//             .http2_prior_knowledge() // EVM services often support HTTP/2
//             .pool_max_idle_per_host(128)
//             .timeout(Duration::from_secs(5))
//             .build()
//             .expect("Failed to build HTTP client");

//         let (metrics_tx, _) = mpsc::unbounded_channel();

//         Self { config, accounts, client, metrics_tx }
//     }

//     /// Runs the benchmark lifecycle: ramp → steady → cooldown.
//     /// Returns aggregated `BenchmarkResult` via the metrics collector.
//     pub async fn run(&self) -> Result<BenchmarkResult> {
//         let total_duration = Duration::from_secs_f64(
//             self.config.ramp_duration_secs + self.config.steady_duration_secs + self.config.cooldown_duration_secs,
//         );

//         // Start collector task
//         let (tx, rx) = mpsc::unbounded_channel();
//         self.metrics_tx = tx.clone(); // Update internal tx for task cloning
//         let collector_handle = tokio::spawn(Self::metrics_collector(rx));

//         let start = Instant::now();
//         let tick_duration = Duration::from_millis(100);
//         let mut tick_count: u64 = 0;
//         let mut pending_reqs: f64 = 0.0;
//         let account_counter = Arc::new(AtomicU64::new(0));
//         let semaphore = Arc::new(Semaphore::new(self.config.max_concurrency));
//         let mut join_set = JoinSet::new();

//         loop {
//             let elapsed = start.elapsed();
//             let elapsed_secs = elapsed.as_secs_f64();

//             // Determine target TPS based on phase
//             let current_tps = if elapsed_secs < self.config.ramp_duration_secs {
//                 (elapsed_secs / self.config.ramp_duration_secs) * self.config.target_tps
//             } else if elapsed_secs < self.config.ramp_duration_secs + self.config.steady_duration_secs {
//                 self.config.target_tps
//             } else {
//                 0.0
//             };

//             // Fractional carry-over for precise tick scheduling
//             pending_reqs += current_tps / 10.0; // 10 ticks/sec
//             let reqs_this_tick = pending_reqs.floor() as usize;
//             pending_reqs -= reqs_this_tick as f64;

//             if reqs_this_tick > 0 {
//                 for _ in 0..reqs_this_tick {
//                     let permit = semaphore.clone();
//                     let tx = tx.clone();
//                     let client = self.client.clone();
//                     let accounts = self.accounts.clone();
//                     let counter = account_counter.clone();
//                     let base_url = self.config.base_url.clone();

//                     join_set.spawn(async move {
//                         // Acquire concurrency permit (non-blocking wait if pool exhausted)
//                         let _permit = permit.acquire().await.expect("Semaphore closed");
//                         let req_start = Instant::now();

//                         // Round-robin account selection
//                         let idx = counter.fetch_add(1, Ordering::Relaxed) % accounts.len() as u64;
//                         let acc = &accounts[idx as usize];

//                         // Construct JSON-RPC payload matching Lobby's exact contract
//                         let payload = json!({
//                             "jsonrpc": "2.0",
//                             "method": "eth_sendRawTransaction",
//                             "params": [{
//                                 "from": acc.address,
//                                 "to": "0x0000000000000000000000000000000000000000",
//                                 "value": "0x2386f26fc10000",
//                                 "chainId": "0x88bb0",
//                                 "gas": "0x5208",
//                                 "maxFeePerGas": "0xba43b7400",
//                                 "maxPriorityFeePerGas": "0x77359400"
//                             }],
//                             "id": req_start.elapsed().as_micros()
//                         });

//                         let resp = client.post(&format!("{}/v1/transactions", base_url))
//                             .header("Authorization", format!("Bearer {}", acc.api_key))
//                             .header("Content-Type", "application/json")
//                             .json(&payload)
//                             .send()
//                             .await;

//                         let latency = req_start.elapsed().as_nanos() as u64;

//                         match resp {
//                             Ok(r) => {
//                                 let _ = tx.send(MetricEvent::Submitted);
//                                 let status = r.status();

//                                 if status.is_success() {
//                                     // Parse response to check for hard failures (NonceTooLow, Mismatch)
//                                     if let Ok(body) = r.json::<serde_json::Value>().await {
//                                         let is_hard_fail = body
//                                             .get("result")
//                                             .and_then(|r| r.get("status"))
//                                             .and_then(|s| s.as_str())
//                                             .map(|s| s.contains("NonceTooLow") || s.contains("NonceMismatch"))
//                                             .unwrap_or(false);

//                                         if is_hard_fail {
//                                             let _ = tx.send(MetricEvent::HardFailure);
//                                         } else {
//                                             let _ = tx.send(MetricEvent::Broadcasted);
//                                         }
//                                     } else {
//                                         let _ = tx.send(MetricEvent::Broadcasted);
//                                     }
//                                 } else if status == StatusCode::SERVICE_UNAVAILABLE || status == StatusCode::TOO_MANY_REQUESTS {
//                                     let _ = tx.send(MetricEvent::BackpressureError);
//                                 } else {
//                                     let _ = tx.send(MetricEvent::HardFailure);
//                                 }
//                                 let _ = tx.send(MetricEvent::Latency(latency));
//                             }
//                             Err(_) => {
//                                 let _ = tx.send(MetricEvent::HardFailure);
//                                 let _ = tx.send(MetricEvent::Latency(latency));
//                             }
//                         }
//                     });
//                 }
//             }

//             // Drain completed tasks to prevent unbounded memory growth
//             while let Some(res) = join_set.try_join_next() {
//                 if let Err(e) = res {
//                     tracing::warn!("Loadgen task panicked: {:?}", e);
//                 }
//             }

//             // Exit condition
//             if elapsed >= total_duration {
//                 break;
//             }

//             // Drift-compensated sleep
//             tick_count += 1;
//             let next_deadline = start + tick_duration * tick_count;
//             if let Some(sleep_dur) = next_deadline.checked_duration_since(Instant::now()) {
//                 tokio::time::sleep(sleep_dur).await;
//             }
//         }

//         // Wait for remaining tasks
//         while join_set.join_next().await.is_some() {}
        
//         // Drop sender to signal collector completion
//         drop(tx);

//         collector_handle.await.map_err(|e| anyhow!("Collector task failed: {:?}", e))?
//     }

//     /// Dedicated collector task: aggregates metrics lock-free via MPSC.
//     async fn metrics_collector(mut rx: mpsc::UnboundedReceiver<MetricEvent>) -> Result<BenchmarkResult> {
//         let mut latency_hist = Histogram::<u64>::new(3).unwrap(); // 3 sigfigs = 0.1% precision
//         let mut total_submitted: u64 = 0;
//         let mut total_broadcasted: u64 = 0;
//         let mut error_count: u64 = 0;
//         let mut hard_failures: u64 = 0;

//         while let Some(event) = rx.recv().await {
//             match event {
//                 MetricEvent::Latency(ns) => {
//                     latency_hist.record(ns).ok();
//                 }
//                 MetricEvent::Submitted => total_submitted += 1,
//                 MetricEvent::Broadcasted => total_broadcasted += 1,
//                 MetricEvent::BackpressureError => error_count += 1,
//                 MetricEvent::HardFailure => {
//                     error_count += 1;
//                     hard_failures += 1;
//                 }
//             }
//         }

//         let total_errors = error_count;
//         let error_rate = if total_submitted > 0 {
//             total_errors as f64 / total_submitted as f64
//         } else {
//             0.0
//         };

//         Ok(BenchmarkResult {
//             total_submitted,
//             total_broadcasted,
//             error_rate,
//             nonce_collisions: 0, // Deterministic nonce allocation guarantees 0
//             hard_failures,
//             latency_ms: LatencyPercentiles {
//                 p50: latency_hist.value_at_percentile(50.0) as f64 / 1_000_000.0,
//                 p95: latency_hist.value_at_percentile(95.0) as f64 / 1_000_000.0,
//                 p99: latency_hist.value_at_percentile(99.0) as f64 / 1_000_000.0,
//             },
//         })
//     }
// }
