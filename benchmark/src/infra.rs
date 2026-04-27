use sqlx::PgPool;
use testcontainers::{ContainerAsync, GenericImage};

/// Central infrastructure context for the benchmark harness.
/// Manages container lifecycles, readiness probes, migrations, and dynamic port resolution.
pub struct InrfaStack {
    pub pg_url: String,
    pub redis_url: String,
    pg_pool: PgPool,
    pg_image: ContainerAsync<GenericImage>,
    redis_image: ContainerAsync<GenericImage>,
}