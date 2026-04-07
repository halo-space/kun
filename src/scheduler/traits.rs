use crate::error::SpiderError;
use crate::scheduler::checkpoint::{Checkpoint, Counts};
use crate::scheduler::runtime::RuntimeEvent;
use crate::scheduler::snapshot::{Overview, Snapshot};
use crate::scheduler::{ClaimedTask, Task, TaskLease, TaskResolution};
use jiff::SignedDuration;

#[allow(async_fn_in_trait)]
/// Coordinates task lifecycle transitions for the engine.
///
/// A scheduler is responsible for moving tasks through the current runtime
/// state buckets: `ready`, `delayed`, and `inflight`.
///
/// It also exposes a unified read surface for scheduler state:
/// `checkpoint()`, `counts()`, `snapshot()`, `scopes()`, `snapshots()`, and
/// `overview()`. Mutable operations such as `pause / resume / release / purge`
/// live on the separate `scheduler::Control` trait.
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

    /// Reads an aggregate overview across every visible scheduler scope.
    async fn overview(&self) -> Result<Overview, SpiderError> {
        self.overview_with_prefix("").await
    }

    /// Reads an aggregate overview across every visible scope whose name
    /// starts with `prefix`.
    async fn overview_with_prefix(&self, prefix: &str) -> Result<Overview, SpiderError> {
        let snapshots = self.snapshots_with_prefix(prefix).await?;
        Ok(Overview::from_snapshots(snapshots))
    }

    /// Takes one ready task for execution and moves it into `inflight`.
    ///
    /// The caller must later resolve it with the returned task lease.
    async fn take_ready(&self) -> Result<Option<ClaimedTask>, SpiderError>;

    /// Takes up to `limit` ready tasks for execution and moves them into
    /// `inflight`.
    ///
    /// This is a throughput convenience API, not a cross-task transaction.
    async fn take_batch_ready(&self, limit: usize) -> Result<Vec<ClaimedTask>, SpiderError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut tasks = Vec::with_capacity(limit);
        while tasks.len() < limit {
            let Some(task) = self.take_ready().await? else {
                break;
            };
            tasks.push(task);
        }
        Ok(tasks)
    }

    /// Marks an inflight task as completed and removes it from the scheduler.
    async fn complete(&self, lease: &TaskLease) -> Result<(), SpiderError>;

    /// Marks a batch of inflight tasks as completed.
    ///
    /// This is a throughput convenience API, not a cross-task transaction.
    async fn complete_batch(&self, leases: Vec<TaskLease>) -> Result<(), SpiderError> {
        for lease in leases {
            self.complete(&lease).await?;
        }
        Ok(())
    }

    /// Atomically completes one inflight task and enqueues follow-up tasks
    /// back into the scheduler when the backend supports it.
    ///
    /// The default implementation falls back to enqueueing tasks first and
    /// then completing the lease.
    async fn complete_and_enqueue(
        &self,
        lease: &TaskLease,
        tasks: Vec<Task>,
    ) -> Result<(), SpiderError> {
        for task in tasks {
            self.enqueue(task).await?;
        }
        self.complete(lease).await
    }

    /// Completes a batch of inflight tasks and enqueues follow-up tasks for
    /// each lease.
    ///
    /// This is a throughput convenience API, not a cross-task transaction.
    async fn complete_and_enqueue_batch(
        &self,
        resolutions: Vec<TaskResolution>,
    ) -> Result<(), SpiderError> {
        for resolution in resolutions {
            self.complete_and_enqueue(&resolution.lease, resolution.tasks)
                .await?;
        }
        Ok(())
    }

    /// Marks an inflight task as not completed and requeues it for later work.
    async fn requeue(&self, lease: &TaskLease) -> Result<(), SpiderError>;

    /// Requeues a batch of inflight tasks for later work.
    ///
    /// This is a throughput convenience API, not a cross-task transaction.
    async fn requeue_batch(&self, leases: Vec<TaskLease>) -> Result<(), SpiderError> {
        for lease in leases {
            self.requeue(&lease).await?;
        }
        Ok(())
    }

    /// Releases inflight tasks currently owned by this scheduler worker back
    /// into the runnable buckets.
    ///
    /// Built-in schedulers use this for graceful worker drain. The default
    /// implementation is a no-op for custom schedulers that do not track
    /// per-worker ownership.
    async fn release_inflight(&self) -> Result<usize, SpiderError> {
        Ok(0)
    }

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

    /// Returns the logical scheduler scope used for runtime observability, if
    /// the backend exposes one.
    fn runtime_scope(&self) -> Option<String> {
        None
    }

    /// Returns the logical worker id used by this scheduler runtime, if the
    /// backend exposes one.
    fn runtime_worker_id(&self) -> Option<String> {
        None
    }

    /// Drains backend-native runtime events accumulated since the previous
    /// drain call.
    fn drain_runtime_events(&self) -> Vec<RuntimeEvent> {
        Vec::new()
    }

    /// Returns whether any task still remains in the scheduler.
    async fn has_pending(&self) -> Result<bool, SpiderError>;
}
