use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use std::sync::Arc;

use kernel::adapter::IntentSink;

use crate::rpc::JsonRpcRequest;

pub fn router(intent_sink: Arc<dyn IntentSink>) -> Router {
    Router::new()
        .route("/", post(handle_rpc))
        .with_state(intent_sink)
}

async fn handle_rpc(
    State(intent_sink): State<Arc<dyn IntentSink>>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    match req.filter(intent_sink).await {
        Ok(resp) => (StatusCode::OK, Json(resp)),
        Err(resp) => (StatusCode::OK, Json(resp)),
    }
}
