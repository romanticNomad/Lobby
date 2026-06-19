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
    Ok(())
}
