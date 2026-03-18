use crate::error::SpiderError;
use crate::scheduler::types::ScheduledTask;

#[allow(async_fn_in_trait)]
pub trait Scheduler: Send + Sync {
    async fn enqueue(&mut self, task: ScheduledTask) -> Result<(), SpiderError>;
    async fn lease(&mut self) -> Result<Option<ScheduledTask>, SpiderError>;
    async fn ack(&mut self, url: &str) -> Result<(), SpiderError>;
    async fn nack(&mut self, url: &str) -> Result<(), SpiderError>;
    async fn has_pending(&self) -> Result<bool, SpiderError>;
}
