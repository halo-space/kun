use crate::request::Request;
use crate::response::Response;
use crate::value::Value;

pub struct CallbackResult {
    pub items: Vec<Value>,
    pub requests: Vec<Request>,
}

impl CallbackResult {
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            requests: Vec::new(),
        }
    }
}

pub trait Spider {
    fn name(&self) -> &str;

    fn start_urls(&self) -> Vec<String> {
        Vec::new()
    }

    fn parse(&self, _response: &Response) -> CallbackResult {
        CallbackResult::empty()
    }
}
