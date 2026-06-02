#[allow(dead_code)]
mod infra;
#[allow(dead_code, unused_imports)]
mod loadgen;
mod metrics;
mod mockrpc;

use anyhow::Ok;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    Ok(())
}
