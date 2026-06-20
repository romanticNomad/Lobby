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

    // 3.
    Ok(())
}
