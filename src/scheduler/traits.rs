use crate::error::SpiderError;
use crate::scheduler::checkpoint::{Checkpoint, Counts};
use crate::scheduler::snapshot::Snapshot;
use crate::scheduler::{ClaimedTask, Task, TaskLease};
use jiff::SignedDuration;

#[allow(async_fn_in_trait)]
/// Coordinates task lifecycle transitions for the engine.
///
/// A scheduler is responsible for moving tasks through the current runtime
/// state buckets: `ready`, `delayed`, and `inflight`.
///
/// It also exposes a unified read surface for scheduler state:
/// `checkpoint()`, `counts()`, `snapshot()`, `scopes()`, and `snapshots()`.
pub trait Scheduler: Send + Sync {
    /// Adds a task into the scheduler buckets.
    async fn enqueue(&self, task: Task) -> Result<(), SpiderError>;

    /// Exports the current scheduler checkpoint.
    async fn checkpoint(&self) -> Result<Checkpoint, SpiderError>;

    /// Returns the current number of tasks tracked in each state bucket.
    async fn counts(&self) -> Result<Counts, SpiderError>;

    /// Reads one runtime snapshot for the current scheduler scope.
    async fn snapshot(&self) -> Result<Snapshot, SpiderError>;

    /// Lists visible scheduler scopes for the current backend.
    async fn scopes(&self) -> Result<Vec<String>, SpiderError> {
        self.scopes_with_prefix("").await
    }

    /// Lists visible scheduler scopes whose names start with `prefix`.
    async fn scopes_with_prefix(&self, prefix: &str) -> Result<Vec<String>, SpiderError> {
        let snapshot = self.snapshot().await?;
        if prefix.is_empty() || snapshot.scope.starts_with(prefix) {
            Ok(vec![snapshot.scope])
        } else {
            Ok(Vec::new())
        }
    }

    /// Reads runtime snapshots for every visible scheduler scope.
    async fn snapshots(&self) -> Result<Vec<Snapshot>, SpiderError> {
        self.snapshots_with_prefix("").await
    }

    /// Reads runtime snapshots for every visible scope whose name starts with
    /// `prefix`.
    async fn snapshots_with_prefix(&self, prefix: &str) -> Result<Vec<Snapshot>, SpiderError> {
        let snapshot = self.snapshot().await?;
        if prefix.is_empty() || snapshot.scope.starts_with(prefix) {
            Ok(vec![snapshot])
        } else {
            Ok(Vec::new())
        }
    }

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

    /// Closes scheduler runtime resources and performs any backend-specific
    /// shutdown cleanup.
    async fn close(&self) -> Result<(), SpiderError> {
        Ok(())
    }

    /// Returns whether any task still remains in the scheduler.
    async fn has_pending(&self) -> Result<bool, SpiderError>;
}
