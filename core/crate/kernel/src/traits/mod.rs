pub mod broadcast;
pub mod cannonicalize;
pub mod nonce;
pub mod sign;
pub mod state;
pub mod validate;

use crate::types::intent::{Intent, IntentError, IntentResult};
use async_trait::async_trait;

#[async_trait]
pub trait Pipeline: Send + Sync + 'static {
    async fn submit(&self, intent: Intent) -> Result<IntentResult, IntentError>;
}
