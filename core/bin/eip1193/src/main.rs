mod handler;
#[allow(dead_code)]
mod protocol;

use axum::{Router, routing::post};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let app = Router::new().route("/", post(handler::rpc_handler));

    let listner = TcpListener::bind("0.0.0.0:8454")
        .await
        .expect("address binding failed!");

    axum::serve(listner, app).await.expect("server failed");
}
