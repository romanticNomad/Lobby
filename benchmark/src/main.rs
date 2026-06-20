#[allow(dead_code)]
mod infra;
#[allow(dead_code, unused_imports)]
mod loadgen;
#[allow(dead_code)]
mod metrics;
#[allow(dead_code)]
mod mockrpc;

use anyhow::{Ok, Result};

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Set up Postgres and Redis
    // 1.1 check health

    // 2. Build test-keys and api-keys

    // 3. Spawn mockrpc_servers

    // 4. Inititate tokio::process for lobby
    // 4.1 Inject env variables
    // 4.2 Launch lobby binary with "benchmark-telemetry" feature flag

    // 5. Start benchark with loadgen::TxTrigger

    // 6. Initiate telemetry_stream_reader task

    // 7. Shutdown after a minute
    // 7.1 tokio::process -> for Lobby
    // 7.2 CancelationToken -> for {mock_rpc, telemetry_stream_reader}
    // 7.3 teardown.await -> test-containers.

    // 8. report() recorded latencies.

    Ok(())
}
