use anyhow::Ok;

#[allow(dead_code)]
mod infra;
#[allow(dead_code)]
mod loadgen;
mod metrics;
mod mockrpc;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    Ok(())
}
