use crate::request::Request;
use crate::response::Response;
use crate::scheduler::TaskId;
use std::sync::Arc;

/// Shared request/response context passed through engine middleware hooks.
#[derive(Debug, Clone)]
pub struct EngineContext {
    pub task_id: TaskId,
    pub request: Request,
    pub response: Option<Response>,
    pub request_origin: Option<String>,
    pub request_started_at: Option<u64>,
    stats: Option<Arc<crate::stats::Tracker>>,
}

impl EngineContext {
    /// Creates a new middleware context for a request.
    pub fn new(request: Request) -> Self {
        Self {
            task_id: TaskId::new(),
            request,
            response: None,
            request_origin: None,
            request_started_at: None,
            stats: None,
        }
    }

    /// Replaces the task identity carried by this context.
    pub fn with_task_id(mut self, task_id: TaskId) -> Self {
        self.task_id = task_id;
        self
    }

    pub(crate) fn with_stats(mut self, stats: Arc<crate::stats::Tracker>) -> Self {
        self.stats = Some(stats);
        self
    }

    pub(crate) fn stats(&self) -> Option<&crate::stats::Tracker> {
        self.stats.as_deref()
    }
}
