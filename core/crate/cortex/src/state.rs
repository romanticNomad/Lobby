use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use dashmap::DashMap;
use kernel::types::ExecutionId;
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

// ============================================================
// tracking pipeline status

/// Coarse-grained lifecycle states that the orchestrator pipeline advances
/// through for each `ExecutionId`.
///
/// The status is written optimistically (no locking beyond DashMap's per-shard
/// locks) — readers may briefly see a stale value, but the transitions are
/// monotonic (states only advance forward or to Failed).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum PipelineStatus {
    /// pipeline semaphore permit aquired
    PermitAquired,
    /// request has been accepted and persisted by RelayHost
    Accepted,
    /// nonce successfully reserved, awaiting signer
    NonceReserved,
    /// transaction signed, awaiting broadcaster
    Signed,
    /// transaction broadcasted, awaiting on-chain confirmation
    Broadcasted {
        #[serde(rename = "tx_hash")]
        tx_hash: String,
    },
    /// Validator confirmed >=1 block confirmation
    Confirmed {
        #[serde(rename = "tx_hash")]
        tx_hash: String,
    },
    /// Pipeline failed at the given stage; the nonce has been released where
    /// applicable.
    Failed { stage: String, reason: String },
}

// ============================================================
// registry

/// Shared, cheaply-cloneable registry of in-flight and recently-completed
/// pipeline statuses.
///
/// Backed by `DashMap` (concurrent HashMap) — no global lock, one lock per
/// shard (64 by default).  Reads and writes are O(1).
#[derive(Clone, Debug)]
pub struct StatusRegistry {
    status_book: Arc<DashMap<ExecutionId, PipelineStatus>>,
}

impl StatusRegistry {
    /// return cortex state book (cheap to clone Dashmap)
    pub fn new() -> Self {
        Self {
            status_book: Arc::new(DashMap::new()),
        }
    }
    /// fn to record or overwrite status of pipeline
    pub fn set(&self, execution_id: ExecutionId, status: PipelineStatus) {
        self.status_book.insert(execution_id, status);
    }
    /// fn to retrieve pipeline status
    pub fn get(&self, execution_id: &ExecutionId) -> Option<PipelineStatus> {
        self.status_book.get(&execution_id).map(|v| v.clone())
    }
}

impl Default for StatusRegistry {
    fn default() -> Self {
        Self {
            status_book: Arc::new(DashMap::new()),
        }
    }
}

// ============================================================
// HTTP response

#[derive(Debug, Serialize)]
pub struct StatusUpdateResponse {
    pub execution_id: String,
    #[serde(flatten)]
    pub status: PipelineStatus,
}

#[derive(Debug, Serialize)]
pub struct StatusErrorResponce {
    error: String,
}

// ============================================================
// axum handler

/// `GET /status/:execution_id`
///
/// Returns the current pipeline status for an execution.  Clients should poll
/// this until status is `confirmed` or `failed`.
///
/// # Responses
/// - `200 OK` — known execution_id, returns `StatusUpdateResponse`
/// - `400 Bad Request` — `execution_id` is not a valid UUID
/// - `404 Not Found` — execution_id is unknown (not yet submitted or expired)
pub async fn get_transaction_status(
    State(registry): State<StatusRegistry>,
    Path(raw_id): Path<String>,
) -> Result<Json<StatusUpdateResponse>, (StatusCode, Json<StatusErrorResponce>)> {
    // parse execution_id
    let uuid = Uuid::parse_str(&raw_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(StatusErrorResponce {
                error: format!("{} is not a valid UUID", raw_id),
            }),
        )
    })?;

    let execution_id = ExecutionId(uuid);

    match registry.get(&execution_id) {
        Some(status) => Ok(Json(StatusUpdateResponse {
            execution_id: raw_id,
            status,
        })),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(StatusErrorResponce {
                error: format!("no pipeline record found the give execution id: {}", raw_id),
            }),
        )),
    }
}

// ============================================================
