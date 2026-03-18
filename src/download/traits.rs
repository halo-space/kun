use crate::error::SpiderError;
use crate::request::Request;
use crate::response::Response;

#[allow(async_fn_in_trait)]
pub trait Downloader: Send + Sync {
    async fn fetch(&self, request: &Request) -> Result<Response, SpiderError>;
}
