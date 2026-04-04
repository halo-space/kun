use crate::error::SpiderError;
use crate::scheduler::{Task, TaskId};

#[allow(async_fn_in_trait)]
/// Coordinates task lifecycle transitions for the engine.
///
/// A scheduler is responsible for moving tasks through the current runtime
/// state buckets: `ready`, `delayed`, and `inflight`.
pub trait Scheduler: Send + Sync {
    /// Adds a task into the scheduler buckets.
    async fn enqueue(&mut self, task: Task) -> Result<(), SpiderError>;

    /// Takes one ready task for execution and moves it into `inflight`.
    ///
    /// The caller must later resolve it with `complete()` or `requeue()`.
    async fn take_ready(&mut self) -> Result<Option<Task>, SpiderError>;

    /// Marks an inflight task as completed and removes it from the scheduler.
    async fn complete(&mut self, task_id: &TaskId) -> Result<(), SpiderError>;

    /// Marks an inflight task as not completed and requeues it for later work.
    async fn requeue(&mut self, task_id: &TaskId) -> Result<(), SpiderError>;

    /// Returns whether any task still remains in the scheduler.
    async fn has_pending(&self) -> Result<bool, SpiderError>;
}
