use crate::middleware::traits::Middleware;
use crate::middleware::types::MiddlewareConfig;

pub struct MiddlewareEntry {
    pub key: String,
    pub config: MiddlewareConfig,
    pub middleware: Box<dyn Middleware>,
}

#[derive(Default)]
pub struct MiddlewareChain {
    pub entries: Vec<MiddlewareEntry>,
}
