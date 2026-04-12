use super::Dedup;
use crate::error::SpiderError;
use crate::request::Request;
use std::future;

/// Dedup implementation that accepts every request.
#[derive(Debug, Default, Clone, Copy)]
pub struct Noop;

impl Dedup for Noop {
    fn check_and_insert(
        &mut self,
        _request: &Request,
    ) -> impl std::future::Future<Output = Result<bool, SpiderError>> + Send {
        future::ready(Ok(true))
    }
}
