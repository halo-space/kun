use crate::error::{SchedulerError, SpiderError};
use crate::scheduler::checkpoint::{Checkpoint, Counts};
use crate::scheduler::snapshot::{InflightTaskSnapshot, Snapshot, WorkerSnapshot};
use crate::scheduler::{ClaimedTask, Scheduler, Task, TaskLease, Worker};
use jiff::{SignedDuration, Timestamp};
use std::cmp::Ordering;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

pub struct Memory {
    scope: String,
    worker: Worker,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    ready: VecDeque<Task>,
    delayed: Vec<Task>,
    inflight: Vec<InflightEntry>,
    worker_last_seen: Option<Timestamp>,
}

#[derive(Clone)]
struct InflightEntry {
    task: Task,
    lease: Option<TaskLease>,
}

static NEXT_MEMORY_SCOPE: AtomicU64 = AtomicU64::new(1);
static NEXT_MEMORY_WORKER: AtomicU64 = AtomicU64::new(1);
static NEXT_MEMORY_LEASE: AtomicU64 = AtomicU64::new(1);

impl Memory {
    pub fn new() -> Self {
        Self {
            scope: next_memory_scope(),
            worker: Worker::new(next_memory_worker_id()),
            state: Mutex::new(State::default()),
        }
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub fn worker_id(&self) -> &str {
        self.worker.worker_id()
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }

    pub fn worker(&self) -> &Worker {
        &self.worker
    }

    pub fn with_worker(mut self, worker: Worker) -> Self {
        self.worker = worker;
        self
    }

    /// Restores a memory scheduler from an existing checkpoint.
    pub fn from_checkpoint(checkpoint: Checkpoint) -> Self {
        Self {
            scope: next_memory_scope(),
            worker: Worker::new(next_memory_worker_id()),
            state: Mutex::new(State {
                ready: VecDeque::from(checkpoint.ready),
                delayed: checkpoint.delayed,
                inflight: checkpoint
                    .inflight
                    .into_iter()
                    .map(InflightEntry::inactive)
                    .collect(),
                worker_last_seen: None,
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
            inflight: state
                .inflight
                .iter()
                .map(|entry| entry.task.clone())
                .collect(),
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
        let mut inflight_task_ids = Vec::new();
        let mut active_lease_count = 0usize;
        let lease_timeout = self.lease_timeout();
        let heartbeat_interval = self.heartbeat_interval();
        for entry in &state.inflight {
            let task = &entry.task;
            let ready_at = task
                .ready_at
                .map(|value| ready_at_timestamp(self.scope.as_str(), task.id.as_str(), value))
                .transpose()?;
            if entry.lease.is_some() {
                inflight_task_ids.push(task.id.clone());
                active_lease_count += 1;
            }
            inflight_tasks.push(InflightTaskSnapshot {
                task_id: task.id.clone(),
                url: task.request.url.clone(),
                worker_id: entry
                    .lease
                    .as_ref()
                    .map(|lease| lease.worker_id().to_string()),
                lease_id: entry
                    .lease
                    .as_ref()
                    .map(|lease| lease.lease_id().to_string()),
                deadline: None,
                priority: task.priority,
                depth: task.depth,
                ready_at,
            });
        }
        let workers = if active_lease_count > 0 {
            let is_stale = match (state.worker_last_seen.as_ref(), lease_timeout) {
                (Some(last_seen), Some(lease_timeout)) => {
                    last_seen.as_millisecond().saturating_add(
                        i64::try_from(lease_timeout.as_millis()).unwrap_or(i64::MAX),
                    ) < Timestamp::now().as_millisecond()
                }
                _ => false,
            };
            vec![WorkerSnapshot {
                worker_id: self.worker_id().to_string(),
                last_seen: state.worker_last_seen,
                is_stale,
                inflight_count: inflight_task_ids.len(),
                active_lease_count,
                inflight_task_ids,
                next_deadline: None,
                lease_timeout,
                heartbeat_interval,
            }]
        } else {
            Vec::new()
        };

        Ok(Snapshot {
            scope: self.scope.clone(),
            counts: Counts {
                ready: state.ready.len(),
                delayed: state.delayed.len(),
                inflight: state.inflight.len(),
            },
            worker_ids: if active_lease_count > 0 {
                vec![self.worker_id().to_string()]
            } else {
                Vec::new()
            },
            active_lease_count,
            deadline_count: 0,
            reclaimed_total: 0,
            reclaimed_in_refresh: 0,
            inflight_tasks,
            workers,
            lease_timeout,
            heartbeat_interval,
        })
    }

    fn lease_timeout(&self) -> Option<SignedDuration> {
        self.worker.effective_lease_timeout(None)
    }

    fn heartbeat_interval(&self) -> Option<SignedDuration> {
        let default = self
            .lease_timeout()
            .map(|timeout| default_heartbeat_interval(timeout.as_millis()));
        self.worker.effective_heartbeat_interval(default)
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

    fn reset_worker_if_idle(&mut self) {
        if self.inflight.iter().all(|entry| entry.lease.is_none()) {
            self.worker_last_seen = None;
        }
    }
}

impl InflightEntry {
    fn inactive(task: Task) -> Self {
        Self { task, lease: None }
    }

    fn active(task: Task, lease: TaskLease) -> Self {
        Self {
            task,
            lease: Some(lease),
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

        let lease = TaskLease::new(
            task.id.clone(),
            self.worker_id().to_string(),
            next_memory_lease_id(self.worker_id()),
        );
        state.worker_last_seen = Some(Timestamp::now());
        state
            .inflight
            .push(InflightEntry::active(task.clone(), lease.clone()));
        Ok(Some(ClaimedTask::new(task, lease)))
    }

    async fn complete(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pos = find_inflight_entry(&state.inflight, lease.task_id()).ok_or_else(|| {
            SpiderError::scheduler(SchedulerError::InactiveLease {
                action: "complete",
                task_id: lease.task_id().as_str().to_string(),
            })
        })?;
        ensure_memory_lease("complete", self.worker_id(), &state.inflight[pos], lease)?;
        state.inflight.remove(pos);
        state.reset_worker_if_idle();
        Ok(())
    }

    async fn complete_and_enqueue(
        &self,
        lease: &TaskLease,
        tasks: Vec<Task>,
    ) -> Result<(), SpiderError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pos = find_inflight_entry(&state.inflight, lease.task_id()).ok_or_else(|| {
            SpiderError::scheduler(SchedulerError::InactiveLease {
                action: "complete",
                task_id: lease.task_id().as_str().to_string(),
            })
        })?;
        ensure_memory_lease("complete", self.worker_id(), &state.inflight[pos], lease)?;
        state.inflight.remove(pos);
        for task in tasks {
            state.push_task(task);
        }
        state.reset_worker_if_idle();
        Ok(())
    }

    async fn requeue(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pos = find_inflight_entry(&state.inflight, lease.task_id()).ok_or_else(|| {
            SpiderError::scheduler(SchedulerError::InactiveLease {
                action: "requeue",
                task_id: lease.task_id().as_str().to_string(),
            })
        })?;
        ensure_memory_lease("requeue", self.worker_id(), &state.inflight[pos], lease)?;
        let entry = state.inflight.remove(pos);
        state.push_task(entry.task);
        state.reset_worker_if_idle();
        Ok(())
    }

    async fn release_inflight(&self) -> Result<usize, SpiderError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let worker_id = self.worker_id().to_string();
        let mut released_tasks = Vec::new();
        let mut kept_inflight = Vec::with_capacity(state.inflight.len());

        for entry in std::mem::take(&mut state.inflight) {
            match entry.lease.as_ref() {
                Some(lease) if lease.worker_id() == worker_id => released_tasks.push(entry.task),
                _ => kept_inflight.push(entry),
            }
        }

        let released = released_tasks.len();
        state.inflight = kept_inflight;
        for task in released_tasks {
            state.push_task(task);
        }
        state.reset_worker_if_idle();
        Ok(released)
    }

    async fn heartbeat(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pos = find_inflight_entry(&state.inflight, lease.task_id()).ok_or_else(|| {
            SpiderError::scheduler(SchedulerError::InactiveLease {
                action: "heartbeat",
                task_id: lease.task_id().as_str().to_string(),
            })
        })?;
        ensure_memory_lease("heartbeat", self.worker_id(), &state.inflight[pos], lease)?;
        state.worker_last_seen = Some(Timestamp::now());
        Ok(())
    }

    fn heartbeat_interval(&self) -> Option<SignedDuration> {
        Memory::heartbeat_interval(self)
    }

    async fn close(&self) -> Result<(), SpiderError> {
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

fn next_memory_worker_id() -> String {
    format!(
        "memory-worker-{}-{}",
        std::process::id(),
        NEXT_MEMORY_WORKER.fetch_add(1, AtomicOrdering::Relaxed)
    )
}

fn next_memory_lease_id(worker_id: &str) -> String {
    format!(
        "{worker_id}-memory-lease-{}-{}",
        Timestamp::now().as_millisecond(),
        NEXT_MEMORY_LEASE.fetch_add(1, AtomicOrdering::Relaxed)
    )
}

fn default_heartbeat_interval(lease_timeout_millis: i128) -> SignedDuration {
    let millis = (lease_timeout_millis / 2).max(1);
    SignedDuration::from_millis(i64::try_from(millis).unwrap_or(i64::MAX))
}

fn find_inflight_entry(
    inflight: &[InflightEntry],
    task_id: &crate::scheduler::TaskId,
) -> Option<usize> {
    inflight.iter().position(|entry| entry.task.id == *task_id)
}

fn ensure_memory_lease(
    action: &'static str,
    current_worker_id: &str,
    entry: &InflightEntry,
    lease: &TaskLease,
) -> Result<(), SpiderError> {
    let Some(active_lease) = entry.lease.as_ref() else {
        return Err(SpiderError::scheduler(SchedulerError::InactiveLease {
            action,
            task_id: lease.task_id().as_str().to_string(),
        }));
    };

    if lease.worker_id() != current_worker_id {
        return Err(SpiderError::scheduler(
            SchedulerError::LeaseWorkerMismatch {
                lease_worker_id: lease.worker_id().to_string(),
                current_worker_id: current_worker_id.to_string(),
            },
        ));
    }

    if lease.lease_id() != active_lease.lease_id() {
        return Err(SpiderError::scheduler(SchedulerError::StaleLease {
            action,
            task_id: lease.task_id().as_str().to_string(),
            worker_id: lease.worker_id().to_string(),
            lease_id: lease.lease_id().to_string(),
        }));
    }

    Ok(())
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
    use crate::error::{SchedulerError, SpiderError};
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
    fn memory_scheduler_complete_and_enqueue_moves_follow_tasks_atomically() {
        let scheduler = Memory::default();
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/current"))))
            .unwrap();

        let claimed = block_on(scheduler.take_ready()).unwrap().unwrap();
        let ready_follow = Task::new(Request::new("https://example.com/follow-ready"));
        let delayed_follow =
            Task::with_delay(Request::new("https://example.com/follow-delayed"), 500);

        block_on(scheduler.complete_and_enqueue(
            &claimed.lease,
            vec![ready_follow.clone(), delayed_follow.clone()],
        ))
        .unwrap();

        assert_eq!(
            scheduler.counts(),
            Counts {
                ready: 1,
                delayed: 1,
                inflight: 0,
            }
        );
        let checkpoint = scheduler.checkpoint();
        assert_eq!(checkpoint.ready.len(), 1);
        assert_eq!(checkpoint.ready[0].id, ready_follow.id);
        assert_eq!(checkpoint.delayed.len(), 1);
        assert_eq!(checkpoint.delayed[0].id, delayed_follow.id);
        assert!(checkpoint.inflight.is_empty());
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
        let scheduler = Memory::default().with_scope("memory:demo").with_worker(
            Worker::new("memory-worker-a")
                .with_lease_timeout(SignedDuration::from_millis(50))
                .with_heartbeat_interval(SignedDuration::from_millis(10)),
        );
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
        assert_eq!(snapshot.worker_ids, vec!["memory-worker-a".to_string()]);
        assert_eq!(snapshot.active_lease_count, 1);
        assert_eq!(snapshot.deadline_count, 0);
        assert_eq!(snapshot.reclaimed_total, 0);
        assert_eq!(snapshot.reclaimed_in_refresh, 0);
        assert_eq!(snapshot.workers.len(), 1);
        assert_eq!(snapshot.inflight_tasks.len(), 1);
        let inflight = &snapshot.inflight_tasks[0];
        assert_eq!(inflight.task_id, claimed.task.id);
        assert_eq!(inflight.url, "https://example.com/inflight");
        assert_eq!(inflight.worker_id.as_deref(), Some("memory-worker-a"));
        assert_eq!(inflight.lease_id.as_deref(), Some(claimed.lease.lease_id()));
        assert_eq!(inflight.deadline, None);
        assert_eq!(inflight.priority, 3);
        assert_eq!(inflight.depth, 1);
        let worker = &snapshot.workers[0];
        assert_eq!(worker.worker_id, "memory-worker-a");
        assert!(worker.last_seen.is_some());
        assert!(!worker.is_stale);
        assert_eq!(worker.inflight_count, 1);
        assert_eq!(worker.active_lease_count, 1);
        assert_eq!(worker.inflight_task_ids, vec![claimed.task.id.clone()]);
        assert_eq!(worker.next_deadline, None);
        assert_eq!(worker.lease_timeout, Some(SignedDuration::from_millis(50)));
        assert_eq!(
            worker.heartbeat_interval,
            Some(SignedDuration::from_millis(10))
        );
    }

    #[test]
    fn memory_scheduler_scopes_and_snapshots_stay_uniform() {
        let scheduler = Memory::default()
            .with_scope("jobs:memory")
            .with_worker(Worker::new("memory-worker-a"));
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/ready")))).unwrap();
        block_on(scheduler.enqueue(Task::with_delay(
            Request::new("https://example.com/delayed"),
            500,
        )))
        .unwrap();
        let _claimed = block_on(scheduler.take_ready()).unwrap().unwrap();

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
        assert_eq!(
            filtered_snapshots[0].counts,
            Counts {
                ready: 0,
                delayed: 1,
                inflight: 1,
            }
        );
        let no_snapshots = block_on(scheduler.snapshots_with_prefix("other:")).unwrap();
        assert!(no_snapshots.is_empty());

        let overview = block_on(scheduler.overview()).unwrap();
        assert_eq!(overview.scope_count, 1);
        assert_eq!(overview.pending_scope_count, 1);
        assert_eq!(overview.stale_scope_count, 0);
        assert_eq!(
            overview.counts,
            Counts {
                ready: 0,
                delayed: 1,
                inflight: 1,
            }
        );
        assert_eq!(overview.worker_count, 1);
        assert_eq!(overview.stale_worker_count, 0);
        assert_eq!(overview.active_lease_count, 1);
        assert_eq!(overview.reclaimed_total, 0);

        let filtered_overview = block_on(scheduler.overview_with_prefix("jobs:")).unwrap();
        assert_eq!(filtered_overview, overview);

        let no_overview = block_on(scheduler.overview_with_prefix("other:")).unwrap();
        assert_eq!(no_overview.scope_count, 0);
        assert_eq!(no_overview.pending_scope_count, 0);
        assert_eq!(no_overview.stale_scope_count, 0);
        assert_eq!(no_overview.counts, Counts::default());
        assert_eq!(no_overview.worker_count, 0);
        assert_eq!(no_overview.stale_worker_count, 0);
        assert_eq!(no_overview.active_lease_count, 0);
        assert_eq!(no_overview.reclaimed_total, 0);
    }

    #[test]
    fn memory_scheduler_rejects_foreign_or_stale_lease_resolution() {
        let scheduler = Memory::default().with_worker(Worker::new("memory-worker-a"));
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/lease")))).unwrap();

        let claimed = block_on(scheduler.take_ready()).unwrap().unwrap();
        let foreign = TaskLease::new(
            claimed.task.id.clone(),
            "memory-worker-b",
            claimed.lease.lease_id(),
        );
        let foreign_error = block_on(scheduler.complete(&foreign)).unwrap_err();
        assert_eq!(
            foreign_error,
            SpiderError::scheduler(SchedulerError::LeaseWorkerMismatch {
                lease_worker_id: "memory-worker-b".to_string(),
                current_worker_id: "memory-worker-a".to_string(),
            })
        );

        let stale = TaskLease::new(claimed.task.id.clone(), "memory-worker-a", "stale-lease");
        let stale_error = block_on(scheduler.heartbeat(&stale)).unwrap_err();
        assert_eq!(
            stale_error,
            SpiderError::scheduler(SchedulerError::StaleLease {
                action: "heartbeat",
                task_id: claimed.task.id.as_str().to_string(),
                worker_id: "memory-worker-a".to_string(),
                lease_id: "stale-lease".to_string(),
            })
        );

        block_on(scheduler.complete(&claimed.lease)).unwrap();
        let inactive_error = block_on(scheduler.complete(&claimed.lease)).unwrap_err();
        assert_eq!(
            inactive_error,
            SpiderError::scheduler(SchedulerError::InactiveLease {
                action: "complete",
                task_id: claimed.task.id.as_str().to_string(),
            })
        );
    }

    #[test]
    fn memory_scheduler_release_inflight_requeues_current_worker_tasks() {
        let scheduler = Memory::default().with_worker(Worker::new("memory-worker-a"));
        block_on(scheduler.enqueue(Task::new(Request::new("https://example.com/release"))))
            .unwrap();

        let claimed = block_on(scheduler.take_ready()).unwrap().unwrap();
        let released = block_on(scheduler.release_inflight()).unwrap();

        assert_eq!(released, 1);
        assert_eq!(
            scheduler.counts(),
            Counts {
                ready: 1,
                delayed: 0,
                inflight: 0,
            }
        );

        let snapshot = block_on(scheduler.snapshot()).unwrap();
        assert!(snapshot.worker_ids.is_empty());
        assert!(snapshot.workers.is_empty());

        let reclaimed = block_on(scheduler.take_ready()).unwrap().unwrap();
        assert_eq!(reclaimed.task.id, claimed.task.id);
    }

    #[test]
    fn memory_scheduler_accepts_shared_worker_config() {
        let worker = Worker::new("memory-worker-a")
            .with_lease_timeout(SignedDuration::from_millis(60))
            .with_heartbeat_interval(SignedDuration::from_millis(15));
        let scheduler = Memory::default().with_worker(worker);

        assert_eq!(scheduler.worker_id(), "memory-worker-a");
        assert_eq!(
            Scheduler::heartbeat_interval(&scheduler),
            Some(SignedDuration::from_millis(15))
        );
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
