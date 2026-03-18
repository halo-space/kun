use crate::download::traits::Downloader;
use crate::error::SpiderError;
use crate::request::{Request, RequestMode};
use crate::response::Response;

#[derive(Default)]
pub struct BrowserDownloader;

impl Downloader for BrowserDownloader {
    async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
        if request.mode != RequestMode::Browser {
            return Err(SpiderError::download(
                "browser downloader received non-browser request",
            ));
        }

        #[cfg(feature = "browser")]
        return fetch_with_chrome(request).await;

        #[cfg(not(feature = "browser"))]
        {
            let mut response = Response::from_request(
                request.clone(),
                200,
                Default::default(),
                Vec::new(),
            );
            response.protocol = Some("browser".to_string());
            response.flags.push("browser".to_string());
            Ok(response)
        }
    }
}

/// Open a URL in the system default browser. Useful for debugging during development.
#[cfg(feature = "open-browser")]
pub fn open_in_system_browser(url: &str) -> Result<(), SpiderError> {
    webbrowser::open(url).map_err(|e| SpiderError::download(e.to_string()))
}

#[cfg(feature = "browser")]
async fn fetch_with_chrome(request: &Request) -> Result<Response, SpiderError> {
    use headless_chrome::LaunchOptions;

    let url = request.url.clone();
    let headless = request
        .browser
        .as_ref()
        .map(|c| c.headless)
        .unwrap_or(true);
    let wait_for = request
        .browser
        .as_ref()
        .and_then(|c| c.wait_for.clone());

    let result = tokio::task::spawn_blocking(move || {
        let launch_options = LaunchOptions {
            headless,
            ..Default::default()
        };

        let browser = headless_chrome::Browser::new(launch_options)
            .map_err(|e| SpiderError::download(e.to_string()))?;

        let tab = browser
            .new_tab()
            .map_err(|e| SpiderError::download(e.to_string()))?;

        tab.navigate_to(&url)
            .map_err(|e| SpiderError::download(e.to_string()))?;

        if let Some(selector) = wait_for {
            tab.wait_for_element(&selector)
                .map_err(|e| SpiderError::download(e.to_string()))?;
        } else {
            tab.wait_until_navigated()
                .map_err(|e| SpiderError::download(e.to_string()))?;
        }

        let content = tab
            .get_content()
            .map_err(|e| SpiderError::download(e.to_string()))?;
        let final_url = tab.get_url();

        Ok::<(String, String), SpiderError>((final_url, content))
    })
    .await
    .map_err(|e| SpiderError::download(e.to_string()))??;

    let (final_url, content) = result;
    let mut response = Response::from_request(
        request.clone(),
        200,
        Default::default(),
        content.into_bytes(),
    );
    response.url = final_url;
    response.protocol = Some("browser".to_string());
    response.flags.push("browser".to_string());
    Ok(response)
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
    fn browser_downloader_rejects_http_request() {
        let downloader = BrowserDownloader;
        let request = Request::new("https://example.com");

        let result = block_on(downloader.fetch(&request));

        assert!(matches!(result, Err(SpiderError::Download(_))));
    }

    #[cfg(not(feature = "browser"))]
    #[test]
    fn browser_downloader_returns_stub_response() {
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
