use crate::types::adapter::{Intent, IntentError, IntentResult};
use async_trait::async_trait;

#[async_trait]
pub trait Pipeline: Send + Sync + 'static {
    async fn submit(&self, intent: Intent) -> Result<IntentResult, IntentError>;
}
