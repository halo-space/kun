use crate::dedup::Dedup;
use crate::error::SpiderError;
use crate::request::Request;

/// Dedup implementation that accepts every request.
#[derive(Debug, Default, Clone, Copy)]
pub struct Noop;

impl Dedup for Noop {
    async fn check_and_insert(&mut self, _request: &Request) -> Result<bool, SpiderError> {
        Ok(true)
    }
}
