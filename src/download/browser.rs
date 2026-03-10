use crate::download::traits::Downloader;
use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::request::Request;
use crate::request::RequestMode;
use crate::response::Response;

#[derive(Default)]
pub struct BrowserDownloader;

impl Downloader for BrowserDownloader {
    fn fetch<'a>(&'a self, request: &'a Request) -> BoxFuture<'a, Result<Response, SpiderError>> {
        Box::pin(async move {
            if request.mode != RequestMode::Browser {
                return Err(SpiderError::download(
                    "browser downloader received non-browser request",
                ));
            }

            let mut response =
                Response::from_request(request.clone(), 200, Default::default(), Vec::new());
            response.protocol = Some("browser".to_string());
            response.flags.push("browser".to_string());
            Ok(response)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::traits::Downloader;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn browser_downloader_returns_response_for_browser_request() {
        let downloader = BrowserDownloader;
        let request = Request::browser("https://example.com");

        let response = block_on(downloader.fetch(&request)).unwrap();

        assert_eq!(response.url, "https://example.com");
        assert_eq!(response.protocol.as_deref(), Some("browser"));
        assert_eq!(response.flags, vec!["browser".to_string()]);
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWake));
        let mut future = Pin::from(Box::new(future));
        let mut context = Context::from_waker(&waker);

        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }
}
