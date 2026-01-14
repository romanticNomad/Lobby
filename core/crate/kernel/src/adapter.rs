use async_trait::async_trait;

pub struct Intent{}
pub struct IntentResult{}
pub struct IntentError{}

#[async_trait]
pub trait IntentSink: Send + Sync + 'static {
    async fn submit(&self, intent: Intent) -> Result<IntentResult, IntentError>;
}
