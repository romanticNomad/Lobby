use dashmap::DashMap;
use kernel::types::{ExecutionId, TxNonce};
use rocksdb::{DB, Options};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

// ============================================================
// registry

/// Shared, cheaply-cloneable registry of in-flight and recently-completed
/// pipeline statuses.
///
/// Backed by:
/// - `DashMap` (concurrent HashMap) for fast in-memory reads (O(1), no disk I/O)
/// - `RocksDB` for crash-safe persistence (write-through on every `set()`)
///
/// **Persistence strategy:**
/// - `set()` writes to both DashMap (immediate) and RocksDB (async via write-ahead log)
/// - `get()` reads from DashMap only (no disk I/O, ~1µs latency)
/// - On boot, RocksDB state is loaded into DashMap (full recovery)
#[derive(Clone, Debug)]
pub struct StatusRegistry {
    pub status_book: Arc<DashMap<ExecutionId, PipelineStatus>>,
    db: Arc<DB>,
}

impl StatusRegistry {
    /// Create a new StatusRegistry with RocksDB persistence.
    ///
    /// # Arguments
    /// - `db_path`: Path to RocksDB directory (will be created if missing)
    ///
    /// # Errors
    /// - Returns `Err` if RocksDB cannot be opened (corrupted DB, permissions, disk full)
    /// - **Caller should panic/exit if this fails** (boot-time only)
    ///
    /// # Recovery
    /// - Loads all existing entries from RocksDB into DashMap
    /// - Logs recovery stats (entry count, elapsed time)
    pub fn new(db_path: PathBuf) -> Result<Self, rocksdb::Error> {
        let start = std::time::Instant::now();

        // RocksDB options tuned for write-heavy workload
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.set_max_open_files(512); // Limit file handles
        opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB write buffer
        opts.set_max_write_buffer_number(3); // Up to 3 memtables
        opts.set_target_file_size_base(64 * 1024 * 1024); // 64MB SST files
        opts.set_compression_type(rocksdb::DBCompressionType::Lz4); // Fast compression

        let db = DB::open(&opts, &db_path)?;
        tracing::info!(?db_path, "rocksdb: booted successfully");

        let status_book = Arc::new(DashMap::new());

        // load existing entries into DB
        let mut loaded_count = 0;
        let iter = db.iterator(rocksdb::IteratorMode::Start);

        for item in iter {
            let (key, value) = item?;

            if key.len() != 16 {
                tracing::warn!("rocksdb: invalid execution_id length, skipping");
                continue;
            }

            let uuid = match Uuid::from_slice(&key) {
                Ok(u) => u,
                Err(e) => {
                    tracing::warn!(%e, "rocksdb: invalid uuid, skipping");
                    continue;
                }
            };
            let execution_id = ExecutionId(uuid);

            match postcard::from_bytes::<PipelineStatus>(&value) {
                Ok(status) => {
                    status_book.insert(execution_id, status);
                    loaded_count += 1;
                }

                Err(e) => {
                    tracing::error!(%execution_id, %e, "rocksdb: status deserialize failed");
                }
            }
        }

        let elapsed = start.elapsed();
        tracing::info!(
            loaded_count,
            elapsed_ms = elapsed.as_millis(),
            "rocksdb: status registry loaded"
        );

        Ok(Self {
            status_book,
            db: Arc::new(db),
        })
    }

    /// Record or overwrite the status of a pipeline.
    ///
    /// Writes to both:
    /// 1. DashMap (immediate, in-memory)
    /// 2. RocksDB (async via write-ahead log, durable)
    ///
    /// **Error handling:**
    /// - DashMap write always succeeds (in-memory)
    /// - RocksDB write failure is logged as ERROR but does NOT crash
    /// - In-memory state remains valid even if disk write fails
    ///
    /// **Why non-crashing?**
    /// - Transient disk errors (full disk, I/O timeout) shouldn't kill the server
    /// - StatusRegistry is observable state, not critical for correctness
    /// - Worst case: Status is lost on restart (client sees "unknown execution_id")
    pub fn set(&self, execution_id: ExecutionId, status: PipelineStatus) {
        // update in-memory
        self.status_book.insert(execution_id, status.clone());

        // persist to rocksdb
        let db = Arc::clone(&self.db);
        tokio::task::spawn_blocking(move || {
            let key = execution_id.0.as_bytes();

            let value = match postcard::to_allocvec(&status) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(%execution_id, %e, "rockdb: failed to serialize status");
                    return;
                }
            };

            if let Err(e) = db.put(key, value) {
                tracing::error!(%execution_id, %e, "failed to write status")
            }
        });
    }
}

// ============================================================
// tracking pipeline status

/// Coarse-grained lifecycle states that the orchestrator pipeline advances
/// through for each `ExecutionId`.
///
/// The status is written optimistically (no locking beyond DashMap's per-shard
/// locks) — readers may briefly see a stale value, but the transitions are
/// monotonic (states only advance forward or to Failed).
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Deserialize)]
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
    /// Syncing nonce on db with the on-chain nonce
    /// retrieved using the given rpc_endpoint
    NonceMismatchDetected {
        nonce_on_chain: TxNonce,
        attempted_nonce: TxNonce,
    },
    /// Validator timed out without confirmation (due to high nonce),
    /// such situation might be created due to nonce gaps
    ValidatorTimedOut { message: String },
}

// ============================================================
// json status reponse

#[derive(Debug, Serialize)]
pub struct JsonStatusResponse {
    pub execution_id: String,
    #[serde(flatten)]
    pub status: PipelineStatus,
}

// ============================================================
