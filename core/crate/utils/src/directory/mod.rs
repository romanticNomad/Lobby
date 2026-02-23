pub mod api;
pub mod rpc;

pub use api::*;
pub use rpc::*;

// ============================================================
/// Parse an environment variable into a given type
pub fn parse_env<T>(key: &'static str, default: T) -> Result<T, T::Err>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match std::env::var(key) {
        Ok(raw) => raw.parse::<T>(),
        Err(_) => Ok(default),
    }
}

// ============================================================
