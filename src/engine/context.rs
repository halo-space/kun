use crate::request::Request;
use crate::response::Response;
use crate::scheduler::TaskId;

/// Shared request/response context passed through engine middleware hooks.
#[derive(Debug, Clone)]
pub struct EngineContext {
    pub task_id: TaskId,
    pub request: Request,
    pub response: Option<Response>,
}

impl EngineContext {
    /// Creates a new middleware context for a request.
    pub fn new(request: Request) -> Self {
        Self {
            task_id: TaskId::new(),
            request,
            response: None,
        }
    }

    /// Replaces the task identity carried by this context.
    pub fn with_task_id(mut self, task_id: TaskId) -> Self {
        self.task_id = task_id;
        self
    }
}
