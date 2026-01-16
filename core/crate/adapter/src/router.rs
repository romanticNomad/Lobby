use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use std::sync::Arc;

use kernel::traits::Pipeline;

use crate::rpc::JsonRpcRequest;

pub fn router(pipeline: Arc<dyn Pipeline>) -> Router {
    Router::new()
        .route("/", post(handle_rpc))
        .with_state(pipeline)
}

async fn handle_rpc(
    State(pipeline): State<Arc<dyn Pipeline>>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    match req.filter(pipeline).await {
        Ok(resp) => (StatusCode::OK, Json(resp)),
        Err(resp) => (StatusCode::OK, Json(resp)),
    }
}
