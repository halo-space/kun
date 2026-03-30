use crate::request::Request;
use crate::response::Response;
use crate::scheduler::types::TaskId;

#[derive(Debug, Clone)]
pub struct EngineContext {
    pub task_id: TaskId,
    pub request: Request,
    pub response: Option<Response>,
}

impl EngineContext {
    pub fn new(request: Request) -> Self {
        Self {
            task_id: TaskId::new(),
            request,
            response: None,
        }
    }

    pub fn with_task_id(mut self, task_id: TaskId) -> Self {
        self.task_id = task_id;
        self
    }

    pub fn with_response(mut self, response: Response) -> Self {
        self.response = Some(response);
        self
    }
}
