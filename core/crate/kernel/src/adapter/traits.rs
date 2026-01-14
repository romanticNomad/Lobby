use async_trait::async_trait;
use super::types::{
    Intent,
    IntentError, IntentResult
};

#[async_trait]
pub trait IntentSink: Send + Sync + 'static {
    async fn submit(&self, intent: Intent) -> Result<IntentResult, IntentError>;
}
