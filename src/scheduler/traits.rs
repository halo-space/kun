use crate::error::SpiderError;
use crate::scheduler::types::ScheduledTask;

pub trait Scheduler {
    fn enqueue(&mut self, task: ScheduledTask) -> Result<(), SpiderError>;
    fn lease(&mut self) -> Result<Option<ScheduledTask>, SpiderError>;
    fn ack(&mut self) -> Result<(), SpiderError>;
    fn nack(&mut self) -> Result<(), SpiderError>;
}
