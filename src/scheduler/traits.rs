use crate::error::SpiderError;
use crate::scheduler::{ClaimedTask, Task, TaskLease};
use jiff::SignedDuration;

#[allow(async_fn_in_trait)]
/// Coordinates task lifecycle transitions for the engine.
///
/// A scheduler is responsible for moving tasks through the current runtime
/// state buckets: `ready`, `delayed`, and `inflight`.
pub trait Scheduler: Send + Sync {
    /// Adds a task into the scheduler buckets.
    async fn enqueue(&self, task: Task) -> Result<(), SpiderError>;

    /// Takes one ready task for execution and moves it into `inflight`.
    ///
    /// The caller must later resolve it with the returned task lease.
    async fn take_ready(&self) -> Result<Option<ClaimedTask>, SpiderError>;

    /// Marks an inflight task as completed and removes it from the scheduler.
    async fn complete(&self, lease: &TaskLease) -> Result<(), SpiderError>;

    /// Marks an inflight task as not completed and requeues it for later work.
    async fn requeue(&self, lease: &TaskLease) -> Result<(), SpiderError>;

    /// Renews the runtime lease for an inflight task when the scheduler
    /// supports explicit heartbeat semantics.
    async fn heartbeat(&self, _lease: &TaskLease) -> Result<(), SpiderError> {
        Ok(())
    }

    /// Returns how often the engine should renew inflight task leases.
    fn heartbeat_interval(&self) -> Option<SignedDuration> {
        None
    }

    /// Returns whether any task still remains in the scheduler.
    async fn has_pending(&self) -> Result<bool, SpiderError>;
}
