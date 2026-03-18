use crate::error::SpiderError;
use crate::scheduler::types::ScheduledTask;

#[allow(async_fn_in_trait)]
pub trait Scheduler: Send {
    async fn enqueue(&mut self, task: ScheduledTask) -> Result<(), SpiderError>;
    async fn lease(&mut self) -> Result<Option<ScheduledTask>, SpiderError>;
    async fn ack(&mut self) -> Result<(), SpiderError>;
    async fn nack(&mut self) -> Result<(), SpiderError>;
    async fn has_pending(&self) -> Result<bool, SpiderError>;
}
