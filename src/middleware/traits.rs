use crate::error::SpiderError;
use crate::future::BoxFuture;

pub trait Middleware: Send + Sync {
    fn process_request<'a>(&'a self) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async { Ok(()) })
    }

    fn process_response<'a>(&'a self) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async { Ok(()) })
    }

    fn process_exception<'a>(
        &'a self,
        _error: &'a SpiderError,
    ) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async { Ok(()) })
    }
}
