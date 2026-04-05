use crate::request::Request;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// Stable identity for a scheduled task.
///
/// This lets the scheduler track tasks independently even when multiple
/// requests share the same URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(String);

impl TaskId {
    /// Creates a new task identity.
    pub fn new() -> Self {
        static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);
        Self(format!(
            "task-{}",
            NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// Returns the string representation of this task identity.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

/// Opaque runtime lease for one claimed task execution.
///
/// Durable schedulers may use this lease to track which worker currently owns
/// an inflight task and whether later heartbeat / complete / requeue calls are
/// still allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskLease {
    task_id: TaskId,
    worker_id: String,
    lease_id: String,
}

impl TaskLease {
    /// Creates a new task lease.
    pub fn new(task_id: TaskId, worker_id: impl Into<String>, lease_id: impl Into<String>) -> Self {
        Self {
            task_id,
            worker_id: worker_id.into(),
            lease_id: lease_id.into(),
        }
    }

    /// Creates a local in-process lease for schedulers without durable worker
    /// ownership semantics.
    pub fn local(task_id: TaskId) -> Self {
        let lease_id = task_id.as_str().to_string();
        Self::new(task_id, "local", lease_id)
    }

    /// Returns the leased task identity.
    pub fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Returns the worker identity currently attached to this lease.
    pub fn worker_id(&self) -> &str {
        self.worker_id.as_str()
    }

    /// Returns the opaque lease token for this claim.
    pub fn lease_id(&self) -> &str {
        self.lease_id.as_str()
    }
}

/// A ready task that has already been claimed by the scheduler.
#[derive(Debug, Clone)]
pub struct ClaimedTask {
    pub task: Task,
    pub lease: TaskLease,
}

impl ClaimedTask {
    pub fn new(task: Task, lease: TaskLease) -> Self {
        Self { task, lease }
    }
}

/// A request plus scheduler-owned execution metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub request: Request,
    pub ready_at: Option<u64>,
    pub priority: i32,
    pub depth: u32,
}

impl Task {
    /// Creates an immediately ready task with a fresh task identity.
    pub fn new(request: Request) -> Self {
        Self::with_id(request, TaskId::new())
    }

    /// Creates an immediately ready task with an explicit task identity.
    pub fn with_id(request: Request, id: TaskId) -> Self {
        Self {
            id,
            request,
            ready_at: None,
            priority: 0,
            depth: 0,
        }
    }

    /// Creates a delayed task with a fresh task identity.
    pub fn with_delay(request: Request, delay: u64) -> Self {
        Self::with_id_and_delay(request, TaskId::new(), delay)
    }

    /// Creates a delayed task with an explicit task identity.
    pub fn with_id_and_delay(request: Request, id: TaskId, delay: u64) -> Self {
        Self {
            id,
            request,
            ready_at: Some(now().saturating_add(delay)),
            priority: 0,
            depth: 0,
        }
    }

    /// Assigns an explicit scheduler priority to this task.
    ///
    /// Higher values are taken before lower values.
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Assigns the crawl depth for this task.
    ///
    /// Lower depth wins when priorities are equal.
    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    /// Returns whether this task is ready to be taken for execution now.
    pub fn is_ready(&self) -> bool {
        self.ready_at.map(|value| value <= now()).unwrap_or(true)
    }
}

fn now() -> u64 {
    u64::try_from(Timestamp::now().as_millisecond()).unwrap_or_default()
}
