use crate::request::Request;
use crate::response::Response;

#[derive(Debug, Default)]
pub struct EngineContext {
    pub request: Option<Request>,
    pub response: Option<Response>,
}
