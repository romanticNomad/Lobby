#[allow(dead_code, unused_imports)]
mod infra;
#[allow(dead_code, unused_imports)]
mod loadgen;
#[allow(dead_code)]
mod metrics;
#[allow(dead_code)]
mod mockrpc;

use crate::infra::InfraStack;
use crate::loadgen::{build_apistack, get_addresses, write_test_keys_json};
use crate::mockrpc::RpcAppState;
use anyhow::{Ok, Result};
use std::path::Path;

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Set up Postgres and Redis

    let infra_stack = InfraStack::build().await?;
    let _pg_pool = infra_stack.get_pool();

    // 2. Build test-keys and api-keys
    write_test_keys_json(100)?;
    let path_test_keys = Path::new("test_keys.json");
    let api_stack = build_apistack(path_test_keys)?;

    // 3. Spawn mockrpc_servers
    let chain_ids = vec![1, 137, 560048];
    let addresses = get_addresses(&api_stack)?;
    let app_state = RpcAppState::new(chain_ids, addresses);
    app_state.spawn_mockrpc_servers().await?;

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
