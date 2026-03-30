use crate::engine::context::EngineContext;
use crate::engine::types::Flow;
use crate::error::SpiderError;
use crate::future::BoxFuture;

pub trait Middleware: Send + Sync {
    fn process_request<'a>(
        &'a self,
        _context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async { Ok(Flow::Continue) })
    }

    fn process_response<'a>(
        &'a self,
        _context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async { Ok(Flow::Continue) })
    }

    fn process_exception<'a>(
        &'a self,
        _context: &'a mut EngineContext,
        _error: &'a SpiderError,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async { Ok(Flow::Continue) })
    }
}
