use std::sync::Arc;

use dashmap::DashMap;
use kernel::types::ExecutionId;
use serde::Serialize;

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
pub enum Pipelinestatus {
    /// request has been accepted and persisted by RelayHost
    Accepted,
    /// nonce successfully reserved, awaiting signer
    NonceReserved,
    /// transaction signed, awaiting broadcaster
    Signed,
    /// transaction broadcasted, awaiting on-chain confirmation
    Broadcasted{
        #[serde(rename = "tx_hash")]
        tx_hash: String
    },
    /// Validator confirmed >=1 block confirmation
    Confirmed {
        #[serde(rename = "tx_hash")]
        tx_hash: String
    },
    /// Pipeline failed at the given stage; the nonce has been released where
    /// applicable.
    Failed {
        stage: String,
        reason: String,
    },
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
    status_book: Arc<DashMap<ExecutionId, Pipelinestatus>>, 
}

impl StatusRegistry {
    pub fn new() -> Self {
        Self { status_book: Arc::new(DashMap::new()) }
    }

    /// fn to record or overwrite status of pipeline
    pub fn set(&self, execution_id: ExecutionId, status: Pipelinestatus) {
        self.status_book.insert(execution_id, status);
    }

    /// fn to retrieve pipeline status
    pub fn get(&self, execution_id: ExecutionId) -> Option<Pipelinestatus> {
        self.status_book.get(&execution_id).map(|v| v.clone())
    }
}

impl Default for StatusRegistry {
    fn default() -> Self {
        Self { status_book: Arc::new(DashMap::new())}
    }
}

// ============================================================
// HTTP response
