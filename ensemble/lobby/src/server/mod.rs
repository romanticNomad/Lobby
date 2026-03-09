pub mod auth;
pub mod handler;
pub mod sweeper;

use cortex::{CortextHandle, artifacts::state::StatusRegistry};
use kernel::types::ApiRegistry;

// ============================================================
// app state

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) api_registry: ApiRegistry,       // authentication
    pub(crate) cortex_handler: CortextHandle,   // POST handler
    pub(crate) status_registry: StatusRegistry, // Get handler
}

impl AppState {
    pub fn new(
        api_registry: ApiRegistry,
        cortex_handler: CortextHandle,
        status_registry: StatusRegistry,
    ) -> Self {
        Self {
            api_registry,
            cortex_handler,
            status_registry,
        }
    }
}

// ============================================================
// defining substate modifications for auth_middleware and get_status, handlers.

impl axum::extract::FromRef<AppState> for StatusRegistry {
    fn from_ref(state: &AppState) -> Self {
        state.status_registry.clone()
    }
}

impl axum::extract::FromRef<AppState> for ApiRegistry {
    fn from_ref(state: &AppState) -> Self {
        state.api_registry.clone()
    }
}

// ============================================================
// DEPRECATED

// /// custom wrapper for `tracing_tree::time::Uptime`.
// /// currently set to 'ms' precision in 's' format.
// pub struct CustomTime {
//     started_at: Instant,
// }

// impl Default for CustomTime {
//     fn default() -> Self {
//         Self {
//             started_at: Instant::now(),
//         }
//     }
// }

// impl FormatTime for CustomTime {
//     fn format_time(&self, w: &mut impl std::fmt::Write) -> std::fmt::Result {
//         let elapsed = self.started_at.elapsed();
//         let secs = elapsed.as_secs_f64();
//         write!(w, " {:.3}s", secs)
//     }

//     fn style_timestamp(
//         &self,
//         ansi: bool,
//         elapsed: Duration,
//         w: &mut impl std::fmt::Write,
//     ) -> std::fmt::Result {
//         let secs = elapsed.as_secs_f64();

//         if ansi {
//             // ANSI codes for dimmed green (similar to default Uptime styling)
//             write!(w, "\x1b[2;32m{:.3}s\x1b[0m", secs)
//         } else {
//             write!(w, " {:.3}s", secs)
//         }
//     }
// }

// // ============================================================
