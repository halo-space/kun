use crate::error::SpiderError;
use crate::scheduler::types::{ScheduledTask, TaskId};

#[allow(async_fn_in_trait)]
pub trait Scheduler: Send + Sync {
    async fn enqueue(&mut self, task: ScheduledTask) -> Result<(), SpiderError>;
    async fn lease(&mut self) -> Result<Option<ScheduledTask>, SpiderError>;
    async fn ack(&mut self, task_id: &TaskId) -> Result<(), SpiderError>;
    async fn nack(&mut self, task_id: &TaskId) -> Result<(), SpiderError>;
    async fn has_pending(&self) -> Result<bool, SpiderError>;
}
