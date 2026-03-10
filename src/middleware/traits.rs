use crate::error::SpiderError;

pub trait Middleware: Send + Sync {
    fn process_request(&self) -> Result<(), SpiderError> {
        Ok(())
    }

    fn process_response(&self) -> Result<(), SpiderError> {
        Ok(())
    }

    fn process_exception(&self, _error: &SpiderError) -> Result<(), SpiderError> {
        Ok(())
    }
}
