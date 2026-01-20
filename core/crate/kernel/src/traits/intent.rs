use crate::types::{
    intent::{Intent, IntentResult},
    validate::ExecutionError,
};
use async_trait::async_trait;

#[async_trait]
pub trait Pipeline: Send + Sync + 'static {
    async fn submit(&self, intent: Intent) -> Result<IntentResult, ExecutionError>;
}
