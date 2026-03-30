use crate::request::Request;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskId(String);

impl TaskId {
    pub fn new() -> Self {
        static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);
        Self(format!(
            "task-{}",
            NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct ScheduledTask {
    pub id: TaskId,
    pub request: Request,
    pub ready_at_ms: Option<u64>,
}

impl ScheduledTask {
    pub fn new(request: Request) -> Self {
        Self::with_task_id(request, TaskId::new())
    }

    pub fn with_task_id(request: Request, id: TaskId) -> Self {
        Self {
            id,
            request,
            ready_at_ms: None,
        }
    }

    pub fn with_delay_ms(request: Request, delay_ms: u64) -> Self {
        Self::with_task_id_and_delay(request, TaskId::new(), delay_ms)
    }

    pub fn with_task_id_and_delay(request: Request, id: TaskId, delay_ms: u64) -> Self {
        Self {
            id,
            request,
            ready_at_ms: Some(now_ms().saturating_add(delay_ms)),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.ready_at_ms
            .map(|value| value <= now_ms())
            .unwrap_or(true)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
