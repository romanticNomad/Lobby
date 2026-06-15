pub mod config;
pub mod error;
pub mod pool;
pub mod retry;
pub mod state;
#[cfg(feature = "benchmark-telemetry")]
pub mod telemetry;

// NO-OP FALLBACK FOR PRODUCTION

#[cfg(not(feature = "benchmark-telemetry"))]
pub mod benchmark {
    use super::*;
    use primitives::types::ExecutionId;

    #[derive(Clone)]
    pub struct TelemetryContext;
    impl TelemetryContext {
        pub fn new() -> Self {
            Self
        }
        #[inline(always)]
        pub fn record_start(&self, _execution_id: ExecutionId) {}
        #[inline(always)]
        pub fn record_stage(&self, _execution_id: ExecutionId, _stage: &'static str) {}
        #[inline(always)]
        pub fn finalize(&self, _execution_id: ExecutionId) {}
    }
}

// ============================================================
// re-exports

pub use config::*;
pub use error::*;
pub use pool::*;
pub use retry::*;
pub use state::*;
#[cfg(feature = "benchmark-telemetry")]
pub use telemetry::*;

// ============================================================
