use crate::error::SpiderError;
use crate::scheduler::checkpoint::{Checkpoint, Counts};
use crate::scheduler::snapshot::{InflightTaskSnapshot, Snapshot};
use crate::scheduler::{ClaimedTask, Scheduler, Task, TaskLease};
use jiff::Timestamp;
use std::cmp::Ordering;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

pub struct Memory {
    scope: String,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    ready: VecDeque<Task>,
    delayed: Vec<Task>,
    inflight: Vec<Task>,
}

static NEXT_MEMORY_SCOPE: AtomicU64 = AtomicU64::new(1);

impl Memory {
    pub fn new() -> Self {
        Self {
            scope: next_memory_scope(),
            state: Mutex::new(State::default()),
        }
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }

    /// Restores a memory scheduler from an existing checkpoint.
    pub fn from_checkpoint(checkpoint: Checkpoint) -> Self {
        Self {
            scope: next_memory_scope(),
            state: Mutex::new(State {
                ready: VecDeque::from(checkpoint.ready),
                delayed: checkpoint.delayed,
                inflight: checkpoint.inflight,
            }),
        }
    }

    /// Exports the current in-memory scheduler checkpoint.
    pub fn checkpoint(&self) -> Checkpoint {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Checkpoint {
            ready: state.ready.iter().cloned().collect(),
            delayed: state.delayed.clone(),
            inflight: state.inflight.clone(),
        }
    }

    /// Returns the number of tasks tracked in each state bucket.
    pub fn counts(&self) -> Counts {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Counts {
            ready: state.ready.len(),
            delayed: state.delayed.len(),
            inflight: state.inflight.len(),
        }
    }

    fn build_snapshot(&self) -> Result<Snapshot, SpiderError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut inflight_tasks = Vec::with_capacity(state.inflight.len());
        for task in &state.inflight {
            let ready_at = task
                .ready_at
                .map(|value| ready_at_timestamp(self.scope.as_str(), task.id.as_str(), value))
                .transpose()?;
            inflight_tasks.push(InflightTaskSnapshot {
                task_id: task.id.clone(),
                url: task.request.url.clone(),
                worker_id: None,
                lease_id: None,
                deadline: None,
                priority: task.priority,
                depth: task.depth,
                ready_at,
            });
        }

        Ok(Snapshot {
            scope: self.scope.clone(),
            counts: Counts {
                ready: state.ready.len(),
                delayed: state.delayed.len(),
                inflight: state.inflight.len(),
            },
            worker_ids: Vec::new(),
            active_lease_count: 0,
            deadline_count: 0,
            reclaimed_total: 0,
            reclaimed_in_refresh: 0,
            inflight_tasks,
            workers: Vec::new(),
            lease_timeout: None,
            heartbeat_interval: None,
        })
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    fn push_task(&mut self, task: Task) {
        if task.is_ready() {
            self.push_ready_task(task);
        } else {
            self.delayed.push(task);
        }
    }

    fn push_ready_task(&mut self, task: Task) {
        let insert_at = self
            .ready
            .iter()
            .position(|existing| ready_ordering(&task, existing) == Ordering::Less)
            .unwrap_or(self.ready.len());

        self.ready.insert(insert_at, task);
    }

    fn promote_delayed(&mut self) {
        let delayed = std::mem::take(&mut self.delayed);

        for task in delayed {
            if task.is_ready() {
                self.push_ready_task(task);
            } else {
                self.delayed.push(task);
            }
        }
    }
}

impl Scheduler for Memory {
    async fn enqueue(&self, task: Task) -> Result<(), SpiderError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.push_task(task);
        Ok(())
    }

    async fn checkpoint(&self) -> Result<Checkpoint, SpiderError> {
        Ok(Memory::checkpoint(self))
    }

    async fn counts(&self) -> Result<Counts, SpiderError> {
        Ok(Memory::counts(self))
    }

    async fn snapshot(&self) -> Result<Snapshot, SpiderError> {
        self.build_snapshot()
    }

    async fn take_ready(&self) -> Result<Option<ClaimedTask>, SpiderError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.promote_delayed();

        let Some(task) = state.ready.pop_front() else {
            return Ok(None);
        };

        let lease = TaskLease::local(task.id.clone());
        state.inflight.push(task.clone());
        Ok(Some(ClaimedTask::new(task, lease)))
    }

    async fn complete(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.inflight.retain(|task| task.id != *lease.task_id());
        Ok(())
    }

    async fn requeue(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(pos) = state
            .inflight
            .iter()
            .position(|task| task.id == *lease.task_id())
        {
            let task = state.inflight.remove(pos);
            state.push_task(task);
        }
        Ok(())
    }

    async fn has_pending(&self) -> Result<bool, SpiderError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(!state.ready.is_empty() || !state.delayed.is_empty() || !state.inflight.is_empty())
    }
}

fn ready_ordering(left: &Task, right: &Task) -> Ordering {
    right
        .priority
        .cmp(&left.priority)
        .then_with(|| left.depth.cmp(&right.depth))
}

fn next_memory_scope() -> String {
    format!(
        "memory:{}:{}",
        std::process::id(),
        NEXT_MEMORY_SCOPE.fetch_add(1, AtomicOrdering::Relaxed)
    )
}

fn ready_at_timestamp(scope: &str, task_id: &str, millis: u64) -> Result<Timestamp, SpiderError> {
    let millis = i64::try_from(millis).map_err(|_| {
        SpiderError::scheduler(format!(
            "memory scheduler snapshot `ready_at` for scope `{scope}` task `{task_id}` exceeds i64 millisecond range"
        ))
    })?;
    Timestamp::from_millisecond(millis).map_err(|error| {
        SpiderError::scheduler(format!(
            "memory scheduler snapshot `ready_at` for scope `{scope}` task `{task_id}` is invalid: {error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use crate::scheduler::Scheduler;
    use jiff::SignedDuration;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn memory_scheduler_supports_async_enqueue_and_take_ready() {
        let scheduler = Memory::default();
        let task = Task::new(Request::new("https://example.com"));

        block_on(scheduler.enqueue(task.clone())).unwrap();
        let taken = block_on(scheduler.take_ready()).unwrap();

        assert_eq!(
            taken.as_ref().map(|task| task.task.id.as_str()),
            Some(task.id.as_str())
        );
        assert_eq!(
            taken.map(|task| task.task.request.url),
            Some(task.request.url)
        );
    }

    #[test]
    fn memory_scheduler_tracks_inflight_until_complete() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/a")))).unwrap();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/b")))).unwrap();

        let first = block_on(scheduler.take_ready()).unwrap();
        let second = block_on(scheduler.take_ready()).unwrap();

        assert_eq!(
            first.as_ref().map(|t| t.task.request.url.as_str()),
            Some("https://example.com/a")
        );
        assert_eq!(
            second.as_ref().map(|t| t.task.request.url.as_str()),
            Some("https://example.com/b")
        );

        assert!(block_on(scheduler.has_pending()).unwrap());

        block_on(scheduler.complete(&first.as_ref().unwrap().lease)).unwrap();
        block_on(scheduler.complete(&second.as_ref().unwrap().lease)).unwrap();

        assert!(!block_on(scheduler.has_pending()).unwrap());
    }

    #[test]
    fn memory_scheduler_requeue_puts_inflight_task_back_to_ready() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/retry")))).unwrap();

        let first = block_on(scheduler.take_ready()).unwrap().unwrap();
        assert_eq!(
            first.task.request.url,
            "https://example.com/retry".to_string()
        );

        block_on(scheduler.requeue(&first.lease)).unwrap();

        let second = block_on(scheduler.take_ready()).unwrap();
        assert_eq!(
            second.map(|task| task.task.request.url),
            Some("https://example.com/retry".to_string())
        );
    }

    #[test]
    fn memory_scheduler_distinguishes_same_url_tasks_by_task_identity() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(
            Request::new("https://example.com/detail").with_method("GET"),
        )))
        .unwrap();
        block_on(
            scheduler.enqueue(Task::new(
                Request::new("https://example.com/detail")
                    .with_method("POST")
                    .with_meta("page", crate::value::Value::Number(2.0)),
            )),
        )
        .unwrap();

        let first = block_on(scheduler.take_ready()).unwrap().unwrap();
        let second = block_on(scheduler.take_ready()).unwrap().unwrap();

        assert_ne!(first.task.id, second.task.id);
        assert_eq!(first.task.request.url, second.task.request.url);

        block_on(scheduler.complete(&first.lease)).unwrap();
        assert!(block_on(scheduler.has_pending()).unwrap());

        block_on(scheduler.complete(&second.lease)).unwrap();
        assert!(!block_on(scheduler.has_pending()).unwrap());
    }

    #[test]
    fn memory_scheduler_skips_delayed_task_until_ready() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::with_delay(
            Request::new("https://example.com/delayed"),
            10,
        )))
        .unwrap();

        let first = block_on(scheduler.take_ready()).unwrap();
        assert!(first.is_none());

        std::thread::sleep(std::time::Duration::try_from(SignedDuration::from_millis(15)).unwrap());

        let second = block_on(scheduler.take_ready()).unwrap();
        assert_eq!(
            second.map(|task| task.task.request.url),
            Some("https://example.com/delayed".to_string())
        );
    }

    #[test]
    fn memory_scheduler_keeps_ready_order_when_delayed_exists() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::with_delay(
            Request::new("https://example.com/delayed"),
            20,
        )))
        .unwrap();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/ready")))).unwrap();

        let first = block_on(scheduler.take_ready()).unwrap();

        assert_eq!(
            first.map(|task| task.task.request.url),
            Some("https://example.com/ready".to_string())
        );
    }

    #[test]
    fn memory_scheduler_prefers_higher_priority_then_lower_depth() {
        let scheduler = Memory::default();
        block_on(
            scheduler.enqueue(
                Task::new(Request::new("https://example.com/depth-2"))
                    .with_priority(0)
                    .with_depth(2),
            ),
        )
        .unwrap();
        block_on(
            scheduler.enqueue(
                Task::new(Request::new("https://example.com/high-priority"))
                    .with_priority(10)
                    .with_depth(8),
            ),
        )
        .unwrap();
        block_on(
            scheduler.enqueue(
                Task::new(Request::new("https://example.com/depth-0"))
                    .with_priority(0)
                    .with_depth(0),
            ),
        )
        .unwrap();

        let first = block_on(scheduler.take_ready()).unwrap().unwrap();
        let second = block_on(scheduler.take_ready()).unwrap().unwrap();
        let third = block_on(scheduler.take_ready()).unwrap().unwrap();

        assert_eq!(first.task.request.url, "https://example.com/high-priority");
        assert_eq!(second.task.request.url, "https://example.com/depth-0");
        assert_eq!(third.task.request.url, "https://example.com/depth-2");
    }

    #[test]
    fn memory_scheduler_keeps_fifo_for_same_priority_and_depth() {
        let scheduler = Memory::default();
        block_on(
            scheduler.enqueue(
                Task::new(Request::new("https://example.com/first"))
                    .with_priority(1)
                    .with_depth(2),
            ),
        )
        .unwrap();
        block_on(
            scheduler.enqueue(
                Task::new(Request::new("https://example.com/second"))
                    .with_priority(1)
                    .with_depth(2),
            ),
        )
        .unwrap();

        let first = block_on(scheduler.take_ready()).unwrap().unwrap();
        let second = block_on(scheduler.take_ready()).unwrap().unwrap();

        assert_eq!(first.task.request.url, "https://example.com/first");
        assert_eq!(second.task.request.url, "https://example.com/second");
    }

    #[test]
    fn memory_scheduler_exposes_scheduler_state_with_explicit_state_buckets() {
        let scheduler = Memory::default();
        let ready = Task::new(Request::new("https://example.com/ready"));
        let delayed = Task::with_delay(Request::new("https://example.com/delayed"), 20);

        block_on(scheduler.enqueue(ready.clone())).unwrap();
        block_on(scheduler.enqueue(delayed.clone())).unwrap();

        let taken = block_on(scheduler.take_ready()).unwrap().unwrap();
        let checkpoint = scheduler.checkpoint();

        assert_eq!(checkpoint.counts().ready, 0);
        assert_eq!(checkpoint.counts().delayed, 1);
        assert_eq!(checkpoint.counts().inflight, 1);
        assert_eq!(checkpoint.counts().total(), 2);
        assert!(checkpoint.has_pending());
        assert_eq!(
            checkpoint.delayed[0].request.url,
            "https://example.com/delayed".to_string()
        );
        assert_eq!(checkpoint.inflight[0].id.as_str(), taken.task.id.as_str());
    }

    #[test]
    fn memory_scheduler_can_restore_from_checkpoint() {
        let ready = Task::new(Request::new("https://example.com/ready"));
        let delayed = Task::with_delay(Request::new("https://example.com/delayed"), 20);
        let inflight = Task::new(Request::new("https://example.com/inflight"));
        let scheduler = Memory::from_checkpoint(Checkpoint {
            ready: vec![ready.clone()],
            delayed: vec![delayed.clone()],
            inflight: vec![inflight.clone()],
        });

        let counts = scheduler.counts();

        assert_eq!(counts.ready, 1);
        assert_eq!(counts.delayed, 1);
        assert_eq!(counts.inflight, 1);
        assert_eq!(counts.total(), 3);
    }

    #[test]
    fn memory_scheduler_snapshot_uses_unified_scheduler_shape() {
        let scheduler = Memory::default().with_scope("memory:demo");
        block_on(
            scheduler.enqueue(
                Task::new(Request::new("https://example.com/inflight"))
                    .with_priority(3)
                    .with_depth(1),
            ),
        )
        .unwrap();
        let claimed = block_on(scheduler.take_ready()).unwrap().unwrap();

        let snapshot = block_on(scheduler.snapshot()).unwrap();

        assert_eq!(snapshot.scope, "memory:demo");
        assert_eq!(
            snapshot.counts,
            Counts {
                ready: 0,
                delayed: 0,
                inflight: 1,
            }
        );
        assert!(snapshot.worker_ids.is_empty());
        assert_eq!(snapshot.active_lease_count, 0);
        assert_eq!(snapshot.deadline_count, 0);
        assert_eq!(snapshot.reclaimed_total, 0);
        assert_eq!(snapshot.reclaimed_in_refresh, 0);
        assert_eq!(snapshot.workers.len(), 0);
        assert_eq!(snapshot.inflight_tasks.len(), 1);
        let inflight = &snapshot.inflight_tasks[0];
        assert_eq!(inflight.task_id, claimed.task.id);
        assert_eq!(inflight.url, "https://example.com/inflight");
        assert_eq!(inflight.worker_id, None);
        assert_eq!(inflight.lease_id, None);
        assert_eq!(inflight.deadline, None);
        assert_eq!(inflight.priority, 3);
        assert_eq!(inflight.depth, 1);
    }

    #[test]
    fn memory_scheduler_scopes_and_snapshots_stay_uniform() {
        let scheduler = Memory::default().with_scope("jobs:memory");

        let scopes = block_on(scheduler.scopes()).unwrap();
        assert_eq!(scopes, vec!["jobs:memory".to_string()]);
        let filtered_scopes = block_on(scheduler.scopes_with_prefix("jobs:")).unwrap();
        assert_eq!(filtered_scopes, vec!["jobs:memory".to_string()]);
        let filtered_empty = block_on(scheduler.scopes_with_prefix("other:")).unwrap();
        assert!(filtered_empty.is_empty());

        let snapshots = block_on(scheduler.snapshots()).unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].scope, "jobs:memory");
        let filtered_snapshots = block_on(scheduler.snapshots_with_prefix("jobs:")).unwrap();
        assert_eq!(filtered_snapshots.len(), 1);
        assert_eq!(filtered_snapshots[0].scope, "jobs:memory");
        let no_snapshots = block_on(scheduler.snapshots_with_prefix("other:")).unwrap();
        assert!(no_snapshots.is_empty());
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWake));
        let mut future = Pin::from(Box::new(future));
        let mut context = Context::from_waker(&waker);

        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }
}
