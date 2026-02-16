use std::{hash::{DefaultHasher, Hash, Hasher}, sync::Arc};

// ============================================================
// building the shard strcut for a generic actor handler

/// A fixed-size pool of `N` actor handles where each handle exclusively owns
/// a shard of the key-space.
///
/// Routing rule: `shard_index = hash(key) % N`
///
/// This gives us:
/// - **True parallelism** — distinct keys land on distinct actors (probability
///   (N-1)/N for any two random keys).
/// - **Sequential ordering** — the same key always routes to the same actor,
///   so state transitions for a given key are never concurrent.
///
/// # Type parameter
/// `T` is the trait object that each actor handle implements.  Handles are
/// stored as `Arc<T>` so they are cheap to clone into pipeline tasks.

pub struct ShardPool<T: ?Sized> {
    shards: Vec<Arc<T>>,
}

// ============================================================
// implimenting shard

impl <T: ?Sized> ShardPool<T> {
    /// Construct a pool from a pre-built `Vec` of handles.
    ///
    /// The caller is responsible for spawning one actor per shard before
    /// calling this constructor (see the `spawn_*_pool` helpers in `lib.rs`).
    ///
    /// # Panics
    /// Panics if `handles` is empty.
    pub fn new(handles: Vec<Arc<T>>) -> Self {
        assert!(!handles.is_empty(), "sharded pool must have at least one shard");
        Self { shards: handles }
    }

    /// Return the handle responsible for `key`.
    ///
    /// Uses `DefaultHasher` (SipHash 1-3) which is good enough for routing;
    /// it is not used for anything security-sensitive here.
    pub fn get<K: Hash>(&self, key: &K) -> Arc<T> {
        Arc::clone(&self.shards[shard_index(key, self.shards.len())])
    }

    /// number of shards in this pool
    #[inline]
    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }
}

// ============================================================
// helper functions

fn shard_index<K: Hash> (key: &K, n:usize ) -> usize {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    (hasher.finish() as usize) % n
}

// ============================================================
