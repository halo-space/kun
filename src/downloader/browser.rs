use crate::downloader::traits::Downloader;
use crate::error::SpiderError;
use crate::request::Request;
use crate::response::Response;

#[derive(Default)]
pub struct BrowserDownloader;

impl Downloader for BrowserDownloader {
    fn fetch(&self, _request: &Request) -> Result<Response, SpiderError> {
        Ok(Response::default())
    }
}
