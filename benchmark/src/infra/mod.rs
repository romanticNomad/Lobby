mod containers;

// ============================================================
// Postgres with benchmark-tuned CLI flags (bypasses default health check)

const PG_CMD: [&str; 13] = [
    "postgres",
    "-c",
    "shared_buffer=512MB",
    "-c",
    "max_connections=100",
    "-c",
    "wal_minimum",
    "-c",
    "fsync=off",
    "-c",
    "synchronous_commit=off",
    "-c",
    "checkpoint_timeout=300s",
];

// ============================================================
//re-exports

pub use containers::InfraStack;