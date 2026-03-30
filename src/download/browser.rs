use crate::download::traits::Downloader;
use crate::error::SpiderError;
use crate::request::browser::Config as BrowserConfig;
use crate::request::{Request, RequestMode};
use crate::response::Response;
#[cfg(feature = "browser")]
use jiff::SignedDuration;
#[cfg(feature = "browser")]
use playwright_rs::protocol::{
    BrowserContextOptions, GotoOptions, Playwright, ProxySettings, Viewport,
};
#[cfg(feature = "browser")]
use serde_json::json;

#[derive(Default)]
pub struct Browser;

impl Downloader for Browser {
    async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
        if request.mode != RequestMode::Browser {
            return Err(SpiderError::download(
                "browser downloader received non-browser request",
            ));
        }

        let config = request
            .browser
            .as_ref()
            .ok_or_else(|| SpiderError::download("browser request is missing browser config"))?;

        validate_browser_request_contract(request, config)?;

        #[cfg(feature = "browser")]
        return fetch_with_playwright(request, config).await;

        #[cfg(not(feature = "browser"))]
        return Err(browser_feature_disabled_error());
    }
}

/// Open a URL in the system default browser. Useful for debugging during development.
#[cfg(feature = "open-browser")]
pub fn open_in_system_browser(url: &str) -> Result<(), SpiderError> {
    webbrowser::open(url).map_err(|e| SpiderError::download(e.to_string()))
}

#[cfg(feature = "browser")]
fn to_std_duration(duration: SignedDuration) -> Result<std::time::Duration, String> {
    std::time::Duration::try_from(duration).map_err(|error| error.to_string())
}

#[cfg(feature = "browser")]
async fn fetch_with_playwright(
    request: &Request,
    config: &BrowserConfig,
) -> Result<Response, SpiderError> {
    let future = fetch_with_playwright_inner(request, config);

    let (final_url, content) = if let Some(timeout) = request.timeout {
        let timeout = to_std_duration(timeout).map_err(|error| {
            SpiderError::download(format!("invalid browser request timeout: {error}"))
        })?;
        tokio::time::timeout(timeout, future).await.map_err(|_| {
            SpiderError::download(format!(
                "browser request timed out after {} ms",
                timeout.as_millis()
            ))
        })??
    } else {
        future.await?
    };

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

#[cfg(not(feature = "browser"))]
fn browser_feature_disabled_error() -> SpiderError {
    SpiderError::download("browser feature is disabled; enable the `browser` feature")
}

fn validate_browser_request_contract(
    request: &Request,
    config: &BrowserConfig,
) -> Result<(), SpiderError> {
    if request.method != "GET" {
        return Err(SpiderError::download(
            "browser request only supports GET navigation on the Playwright route",
        ));
    }

    if request.body.is_some() {
        return Err(SpiderError::download(
            "browser request body is not implemented yet on the Playwright route",
        ));
    }

    if config.stealth {
        return Err(SpiderError::download(
            "browser stealth is not implemented yet on the Playwright route",
        ));
    }

    if config.fingerprint_profile.is_some() {
        return Err(SpiderError::download(
            "browser fingerprint_profile is not implemented yet on the Playwright route",
        ));
    }

    if request.session.is_some() {
        return Err(SpiderError::download(
            "browser session is not implemented yet on the Playwright route",
        ));
    }

    Ok(())
}

#[cfg(feature = "browser")]
async fn fetch_with_playwright_inner(
    request: &Request,
    config: &BrowserConfig,
) -> Result<(String, String), SpiderError> {
    let playwright = Playwright::launch().await.map_err(map_playwright_error)?;
    let options = build_context_options(config, request, request.timeout)?;
    let user_data_dir = TemporaryUserDataDir::new();
    let user_data_path = user_data_dir.path();

    let context = match config.engine {
        crate::request::browser::Engine::Chromium => {
            playwright
                .chromium()
                .launch_persistent_context_with_options(user_data_path.clone(), options)
                .await
        }
        crate::request::browser::Engine::Firefox => {
            playwright
                .firefox()
                .launch_persistent_context_with_options(user_data_path.clone(), options)
                .await
        }
        crate::request::browser::Engine::Webkit => {
            playwright
                .webkit()
                .launch_persistent_context_with_options(user_data_path.clone(), options)
                .await
        }
    }
    .map_err(map_playwright_error)?;

    let result = async {
        let page = context.new_page().await.map_err(map_playwright_error)?;
        let goto = request
            .timeout
            .map(|timeout| {
                to_std_duration(timeout)
                    .map(|duration| GotoOptions::default().timeout(duration))
                    .map_err(|error| {
                        SpiderError::download(format!("invalid browser request timeout: {error}"))
                    })
            })
            .transpose()?;
        let _ = page
            .goto(&request.url, goto)
            .await
            .map_err(map_playwright_error)?;

        if let Some(selector) = &config.wait_for {
            let frame = page.main_frame().await.map_err(map_playwright_error)?;
            wait_for_selector(&frame, selector, request.timeout).await?;
        }

        let final_url = page.url();
        let content = page.content().await.map_err(map_playwright_error)?;

        Ok::<(String, String), SpiderError>((final_url, content))
    }
    .await;

    context.close().await.map_err(map_playwright_error)?;
    result
}

#[cfg(feature = "browser")]
fn build_context_options(
    config: &BrowserConfig,
    request: &Request,
    timeout: Option<SignedDuration>,
) -> Result<BrowserContextOptions, SpiderError> {
    let mut builder = BrowserContextOptions::builder()
        .headless(config.headless)
        .viewport(Viewport {
            width: config.viewport.width,
            height: config.viewport.height,
        });

    if let Some(timeout) = timeout {
        let timeout = to_std_duration(timeout)
            .map_err(|error| SpiderError::download(format!("invalid browser timeout: {error}")))?;
        builder = builder.timeout(timeout.as_millis() as f64);
    }

    if let Some(proxy) = &request.proxy {
        builder = builder.proxy(ProxySettings {
            server: proxy.url.clone(),
            bypass: None,
            username: None,
            password: None,
        });
    }

    let headers = request
        .headers
        .iter()
        .map(|(key, values)| (key.clone(), values.join(", ")))
        .collect();
    if !request.headers.is_empty() {
        builder = builder.extra_http_headers(headers);
    }

    Ok(builder.build())
}

#[cfg(feature = "browser")]
async fn wait_for_selector(
    frame: &playwright_rs::protocol::Frame,
    selector: &str,
    request_timeout: Option<SignedDuration>,
) -> Result<(), SpiderError> {
    let timeout = request_timeout.unwrap_or(SignedDuration::from_secs(30));
    let timeout = to_std_duration(timeout)
        .map_err(|error| SpiderError::download(format!("invalid browser wait timeout: {error}")))?;
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let found = frame
            .evaluate(
                "(arg) => document.querySelector(arg.selector) !== null",
                Some(&json!({ "selector": selector })),
            )
            .await
            .map_err(map_playwright_error)?;

        if found.as_bool() == Some(true) {
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(SpiderError::download(format!(
                "browser wait_for selector timed out: {selector}"
            )));
        }

        tokio::time::sleep(to_std_duration(SignedDuration::from_millis(100)).unwrap()).await;
    }
}

#[cfg(feature = "browser")]
fn map_playwright_error(error: impl std::fmt::Display) -> SpiderError {
    SpiderError::download(format!("playwright error: {error}"))
}

#[cfg(feature = "browser")]
struct TemporaryUserDataDir {
    path: std::path::PathBuf,
}

#[cfg(feature = "browser")]
impl TemporaryUserDataDir {
    fn new() -> Self {
        let unique_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "halo-spider-playwright-{}-{unique_id}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&path);

        Self { path }
    }

    fn path(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

#[cfg(feature = "browser")]
impl Drop for TemporaryUserDataDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::traits::Downloader;
    use crate::request::browser::Config as BrowserConfig;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn browser_downloader_rejects_http_request() {
        let downloader = Browser;
        let request = Request::new("https://example.com");

        let result = block_on(downloader.fetch(&request));

        assert!(matches!(result, Err(SpiderError::Download(_))));
    }

    #[test]
    fn browser_request_contract_rejects_stealth() {
        let request = Request::browser("https://example.com")
            .with_browser(BrowserConfig::default().with_stealth(true));

        let error = validate_browser_request_contract(
            &request,
            request
                .browser
                .as_ref()
                .expect("browser config should exist"),
        )
        .unwrap_err();

        assert_eq!(
            error,
            SpiderError::download("browser stealth is not implemented yet on the Playwright route",)
        );
    }

    #[test]
    fn browser_request_contract_rejects_fingerprint_profile() {
        let request = Request::browser("https://example.com")
            .with_browser(BrowserConfig::default().with_fingerprint_profile("desktop"));

        let error = validate_browser_request_contract(
            &request,
            request
                .browser
                .as_ref()
                .expect("browser config should exist"),
        )
        .unwrap_err();

        assert_eq!(
            error,
            SpiderError::download(
                "browser fingerprint_profile is not implemented yet on the Playwright route",
            )
        );
    }

    #[test]
    fn browser_request_contract_rejects_non_get_request() {
        let request = Request::browser("https://example.com").with_method("POST");

        let error = validate_browser_request_contract(
            &request,
            request
                .browser
                .as_ref()
                .expect("browser config should exist"),
        )
        .unwrap_err();

        assert_eq!(
            error,
            SpiderError::download(
                "browser request only supports GET navigation on the Playwright route",
            )
        );
    }

    #[cfg(not(feature = "browser"))]
    #[test]
    fn browser_downloader_fails_explicitly_when_feature_is_disabled() {
        let downloader = Browser;
        let request = Request::browser("https://example.com");

        let error = block_on(downloader.fetch(&request)).unwrap_err();

        assert_eq!(error, browser_feature_disabled_error());
    }

    #[cfg(feature = "browser")]
    #[test]
    fn browser_downloader_rejects_unsupported_config_before_launch() {
        let downloader = Browser;
        let request = Request::browser("https://example.com")
            .with_browser(BrowserConfig::default().with_stealth(true));

        let error = block_on(downloader.fetch(&request)).unwrap_err();

        assert_eq!(
            error,
            SpiderError::download("browser stealth is not implemented yet on the Playwright route",)
        );
    }

    #[cfg(feature = "browser")]
    #[test]
    fn build_context_options_matches_browser_contract() {
        let config = BrowserConfig::default()
            .with_headless(false)
            .with_viewport(1440, 900);
        let request = Request::browser("https://example.com")
            .with_header("x-token", "abc")
            .with_proxy("http://127.0.0.1:8080");
        let options =
            build_context_options(&config, &request, Some(SignedDuration::from_secs(8))).unwrap();

        assert_eq!(options.headless, Some(false));
        assert_eq!(
            options.viewport.as_ref().map(|viewport| viewport.width),
            Some(1440)
        );
        assert_eq!(
            options.viewport.as_ref().map(|viewport| viewport.height),
            Some(900)
        );
        assert_eq!(options.timeout, Some(8000.0));
        assert_eq!(
            options
                .extra_http_headers
                .as_ref()
                .and_then(|headers| headers.get("x-token"))
                .map(String::as_str),
            Some("abc")
        );
        assert_eq!(
            options.proxy.as_ref().map(|proxy| proxy.server.as_str()),
            Some("http://127.0.0.1:8080")
        );
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
