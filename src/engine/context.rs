use crate::request::Request;
use crate::response::Response;

#[derive(Debug, Clone)]
pub struct EngineContext {
    pub request: Request,
    pub response: Option<Response>,
}

impl EngineContext {
    pub fn new(request: Request) -> Self {
        Self {
            request,
            response: None,
        }
    }

    pub fn with_response(mut self, response: Response) -> Self {
        self.response = Some(response);
        self
    }
}
