use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::scheduler::types::ScheduledTask;

pub trait Scheduler: Send {
    fn enqueue<'a>(&'a mut self, task: ScheduledTask) -> BoxFuture<'a, Result<(), SpiderError>>;
    fn lease<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<ScheduledTask>, SpiderError>>;
    fn ack<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SpiderError>>;
    fn nack<'a>(&'a mut self) -> BoxFuture<'a, Result<(), SpiderError>>;
}
