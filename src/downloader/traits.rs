use crate::error::SpiderError;
use crate::request::Request;
use crate::response::Response;

pub trait Downloader {
    fn fetch(&self, request: &Request) -> Result<Response, SpiderError>;
}
