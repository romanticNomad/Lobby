pub mod auth;
pub mod post;

use crate::relayhost::handle::RelayHostHandle;
use axum::{Router, middleware, routing::post};
use dashmap::DashMap;
use kernel::types::{ApiKey, ClientConfig};
use std::{net::SocketAddr, sync::Arc};
use tower_http::trace::TraceLayer;
use tracing::info;

// ============================================================
// middleware - app state

#[derive(Clone)]
pub struct AppState {
    api_registry: Arc<DashMap<ApiKey, ClientConfig>>,
    relayhost_handle: RelayHostHandle,
}

impl AppState {
    pub fn new(
        api_registry: Arc<DashMap<ApiKey, ClientConfig>>,
        relayhost_handle: RelayHostHandle,
    ) -> Self {
        Self {
            api_registry,
            relayhost_handle,
        }
    }
}

// ============================================================
// build axum app router

pub fn build_app(state: AppState) -> Router {
    Router::new()
        .route("v1/transactions", post(post::submit_transaction))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::auth_middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

// ============================================================
// start HTTP-server

pub async fn serve(app: Router, server_addr: SocketAddr) -> Result<(), std::io::Error> {
    info!("starting lobby server on: {}", server_addr);

    let listner = tokio::net::TcpListener::bind(server_addr).await?;
    axum::serve(listner, app).await?;

    Ok(())
}

// ============================================================
