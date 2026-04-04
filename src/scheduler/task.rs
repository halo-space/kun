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

/// A request plus scheduler-owned execution metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub request: Request,
    pub ready_at_ms: Option<u64>,
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
            ready_at_ms: None,
            priority: 0,
            depth: 0,
        }
    }

    /// Creates a delayed task with a fresh task identity.
    pub fn with_delay_ms(request: Request, delay_ms: u64) -> Self {
        Self::with_id_and_delay(request, TaskId::new(), delay_ms)
    }

    /// Creates a delayed task with an explicit task identity.
    pub fn with_id_and_delay(request: Request, id: TaskId, delay_ms: u64) -> Self {
        Self {
            id,
            request,
            ready_at_ms: Some(now_ms().saturating_add(delay_ms)),
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
        self.ready_at_ms
            .map(|value| value <= now_ms())
            .unwrap_or(true)
    }
}

fn now_ms() -> u64 {
    u64::try_from(Timestamp::now().as_millisecond()).unwrap_or_default()
}
