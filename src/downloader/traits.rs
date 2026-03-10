use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::request::Request;
use crate::response::Response;

pub trait Downloader: Send + Sync {
    fn fetch<'a>(&'a self, request: &'a Request) -> BoxFuture<'a, Result<Response, SpiderError>>;
}
