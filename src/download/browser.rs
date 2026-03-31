use crate::download::traits::Downloader;
use crate::error::SpiderError;
#[cfg(any(feature = "browser", test))]
use crate::request::Headers;
use crate::request::browser::Config as BrowserConfig;
use crate::request::{Request, RequestMode};
use crate::response::Response;
#[cfg(feature = "browser")]
use jiff::SignedDuration;
#[cfg(feature = "browser")]
use playwright_rs::protocol::{
    BrowserContextOptions, ContinueOptions, GotoOptions, Playwright, ProxySettings, Viewport,
};
#[cfg(feature = "browser")]
use serde_json::json;
#[cfg(any(feature = "browser", test))]
use std::path::PathBuf;
#[cfg(feature = "browser")]
use std::sync::Arc;
#[cfg(feature = "browser")]
use std::sync::atomic::{AtomicBool, Ordering};

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

    let result = if let Some(timeout) = request.timeout {
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

    Ok(build_browser_response(request, result))
}

#[cfg(not(feature = "browser"))]
fn browser_feature_disabled_error() -> SpiderError {
    SpiderError::download("browser feature is disabled; enable the `browser` feature")
}

fn validate_browser_request_contract(
    _request: &Request,
    config: &BrowserConfig,
) -> Result<(), SpiderError> {
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

    Ok(())
}

#[cfg(any(feature = "browser", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserNavigationRequestOverride {
    url: String,
    method: String,
    body: Option<Vec<u8>>,
}

#[cfg(any(feature = "browser", test))]
impl BrowserNavigationRequestOverride {
    fn for_request(request: &Request) -> Option<Self> {
        if request.method.eq_ignore_ascii_case("GET") && request.body.is_none() {
            return None;
        }

        Some(Self {
            url: request.url.clone(),
            method: request.method.clone(),
            body: request.body.clone(),
        })
    }

    fn matches(&self, url: &str, is_navigation_request: bool) -> bool {
        is_navigation_request && url == self.url
    }

    #[cfg(feature = "browser")]
    fn to_continue_options(&self) -> ContinueOptions {
        let mut builder = ContinueOptions::builder().method(self.method.clone());

        if let Some(body) = &self.body {
            builder = builder.post_data_bytes(body.clone());
        }

        builder.build()
    }
}

#[cfg(any(feature = "browser", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserResponseMetadata {
    status: u16,
    headers: Headers,
    protocol: Option<String>,
}

#[cfg(any(feature = "browser", test))]
impl Default for BrowserResponseMetadata {
    fn default() -> Self {
        Self {
            // Some browser navigations, such as `about:blank` or `data:` URLs, do not produce
            // a network response. Use 0 to indicate "no navigation response available" rather
            // than fabricating a successful HTTP status.
            status: 0,
            headers: Headers::new(),
            protocol: Some("browser".to_string()),
        }
    }
}

#[cfg(any(feature = "browser", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserFetchResult {
    final_url: String,
    content: String,
    metadata: BrowserResponseMetadata,
}

#[cfg(any(feature = "browser", test))]
fn build_browser_response(request: &Request, result: BrowserFetchResult) -> Response {
    let mut response = Response::from_request(
        request.clone(),
        result.metadata.status,
        result.metadata.headers,
        result.content.into_bytes(),
    );
    response.url = result.final_url;
    response.protocol = result.metadata.protocol;
    response.flags.push("browser".to_string());
    response
}

#[cfg(any(feature = "browser", test))]
fn collect_browser_response_headers(
    entries: impl IntoIterator<Item = (String, String)>,
) -> Headers {
    let mut headers = Headers::new();

    for (name, value) in entries {
        headers.entry(name).or_default().push(value);
    }

    headers
}

#[cfg(feature = "browser")]
async fn browser_response_metadata(
    navigation_response: Option<&playwright_rs::protocol::page::Response>,
) -> Result<BrowserResponseMetadata, SpiderError> {
    let Some(navigation_response) = navigation_response else {
        return Ok(BrowserResponseMetadata::default());
    };

    let header_entries = navigation_response
        .headers_array()
        .await
        .map_err(map_playwright_error)?;

    Ok(BrowserResponseMetadata {
        status: navigation_response.status(),
        headers: collect_browser_response_headers(
            header_entries
                .into_iter()
                .map(|entry| (entry.name, entry.value)),
        ),
        // `Response.protocol` in browser mode continues to describe the browser execution path.
        // Playwright's current navigation response API does not expose an HTTP version value that
        // would align with the HTTP downloader's `HTTP/1.1` style protocol string.
        protocol: Some("browser".to_string()),
    })
}

#[cfg(feature = "browser")]
async fn fetch_with_playwright_inner(
    request: &Request,
    config: &BrowserConfig,
) -> Result<BrowserFetchResult, SpiderError> {
    let playwright = Playwright::launch().await.map_err(map_playwright_error)?;
    let options = build_context_options(config, request, request.timeout)?;
    let navigation_request_override = BrowserNavigationRequestOverride::for_request(request);
    let user_data_dir = BrowserUserDataDir::for_request(request)?;
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
        if let Some(navigation_request_override) = navigation_request_override.clone() {
            let navigation_override_applied = Arc::new(AtomicBool::new(false));
            let navigation_override_state = Arc::clone(&navigation_override_applied);

            context
                .route("**", move |route| {
                    let navigation_request_override = navigation_request_override.clone();
                    let navigation_override_state = Arc::clone(&navigation_override_state);

                    async move {
                        let intercepted_request = route.request();

                        if navigation_request_override.matches(
                            intercepted_request.url(),
                            intercepted_request.is_navigation_request(),
                        ) && !navigation_override_state.swap(true, Ordering::SeqCst)
                        {
                            return route
                                .continue_(Some(navigation_request_override.to_continue_options()))
                                .await;
                        }

                        route.continue_(None).await
                    }
                })
                .await
                .map_err(map_playwright_error)?;
        }

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
        let navigation_response = page
            .goto(&request.url, goto)
            .await
            .map_err(map_playwright_error)?;

        if let Some(selector) = &config.wait_for {
            let frame = page.main_frame().await.map_err(map_playwright_error)?;
            wait_for_selector(&frame, selector, request.timeout).await?;
        }

        let final_url = page.url();
        let content = page.content().await.map_err(map_playwright_error)?;
        let metadata = browser_response_metadata(navigation_response.as_ref()).await?;

        Ok::<BrowserFetchResult, SpiderError>(BrowserFetchResult {
            final_url,
            content,
            metadata,
        })
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
enum BrowserUserDataDir {
    Temporary(TemporaryUserDataDir),
    Persistent(PathBuf),
}

#[cfg(feature = "browser")]
impl BrowserUserDataDir {
    fn for_request(request: &Request) -> Result<Self, SpiderError> {
        let Some(session) = &request.session else {
            return Ok(Self::Temporary(TemporaryUserDataDir::new()));
        };

        let path = browser_session_user_data_dir(&session.id);
        std::fs::create_dir_all(&path).map_err(|error| {
            SpiderError::download(format!(
                "failed to create browser session user data dir: {error}"
            ))
        })?;

        Ok(Self::Persistent(path))
    }

    fn path(&self) -> String {
        match self {
            Self::Temporary(dir) => dir.path(),
            Self::Persistent(path) => path.to_string_lossy().into_owned(),
        }
    }
}

#[cfg(any(feature = "browser", test))]
fn browser_session_user_data_dir(session_id: &str) -> PathBuf {
    std::env::temp_dir()
        .join("halo-spider-playwright-sessions")
        .join(hex_encode(session_id.as_bytes()))
}

#[cfg(any(feature = "browser", test))]
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }

    output
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
    fn browser_request_contract_allows_non_get_request() {
        let request = Request::browser("https://example.com").with_method("POST");

        let result = validate_browser_request_contract(
            &request,
            request
                .browser
                .as_ref()
                .expect("browser config should exist"),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn browser_request_contract_allows_request_body() {
        let request = Request::browser("https://example.com").with_body("payload");

        let result = validate_browser_request_contract(
            &request,
            request
                .browser
                .as_ref()
                .expect("browser config should exist"),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn browser_request_contract_allows_session() {
        let request = Request::browser("https://example.com").with_session("shared-browser");

        let result = validate_browser_request_contract(
            &request,
            request
                .browser
                .as_ref()
                .expect("browser config should exist"),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn browser_navigation_request_override_is_absent_for_default_get_without_body() {
        let request = Request::browser("https://example.com");

        let override_request = BrowserNavigationRequestOverride::for_request(&request);

        assert_eq!(override_request, None);
    }

    #[test]
    fn browser_navigation_request_override_captures_method_and_body() {
        let request = Request::browser("https://example.com")
            .with_method("POST")
            .with_body("payload");

        let override_request = BrowserNavigationRequestOverride::for_request(&request)
            .expect("navigation override should exist");

        assert_eq!(override_request.url, "https://example.com");
        assert_eq!(override_request.method, "POST");
        assert_eq!(override_request.body, Some(b"payload".to_vec()));
        assert!(override_request.matches("https://example.com", true));
        assert!(!override_request.matches("https://example.com/other", true));
        assert!(!override_request.matches("https://example.com", false));
    }

    #[cfg(feature = "browser")]
    #[test]
    fn browser_navigation_request_override_builds_continue_options() {
        let override_request = BrowserNavigationRequestOverride {
            url: "https://example.com".to_string(),
            method: "POST".to_string(),
            body: Some(b"payload".to_vec()),
        };

        let continue_options = override_request.to_continue_options();

        assert_eq!(continue_options.method.as_deref(), Some("POST"));
        assert_eq!(continue_options.post_data_bytes, Some(b"payload".to_vec()));
        assert_eq!(continue_options.post_data, None);
    }

    #[test]
    fn collect_browser_response_headers_preserves_duplicate_header_entries() {
        let headers = collect_browser_response_headers([
            ("set-cookie".to_string(), "sid=1".to_string()),
            ("set-cookie".to_string(), "lang=zh".to_string()),
            ("content-type".to_string(), "text/html".to_string()),
        ]);

        assert_eq!(
            headers.get("set-cookie"),
            Some(&vec!["sid=1".to_string(), "lang=zh".to_string()])
        );
        assert_eq!(
            headers.get("content-type"),
            Some(&vec!["text/html".to_string()])
        );
    }

    #[test]
    fn build_browser_response_uses_real_navigation_metadata() {
        let request = Request::browser("https://example.com/list");
        let result = BrowserFetchResult {
            final_url: "https://example.com/detail".to_string(),
            content: "<html>detail</html>".to_string(),
            metadata: BrowserResponseMetadata {
                status: 302,
                headers: collect_browser_response_headers([
                    (
                        "location".to_string(),
                        "https://example.com/detail".to_string(),
                    ),
                    ("content-type".to_string(), "text/html".to_string()),
                ]),
                protocol: Some("browser".to_string()),
            },
        };

        let response = build_browser_response(&request, result);

        assert_eq!(response.url, "https://example.com/detail");
        assert_eq!(response.status, 302);
        assert_eq!(
            response.headers.get("location"),
            Some(&vec!["https://example.com/detail".to_string()])
        );
        assert_eq!(response.text, "<html>detail</html>");
        assert_eq!(response.protocol.as_deref(), Some("browser"));
        assert_eq!(response.flags, vec!["browser".to_string()]);
        assert!(response.ip_address.is_none());
        assert!(response.certificate.is_none());
    }

    #[test]
    fn build_browser_response_keeps_limited_metadata_when_no_navigation_response_exists() {
        let request = Request::browser("about:blank");
        let result = BrowserFetchResult {
            final_url: "about:blank".to_string(),
            content: "<html></html>".to_string(),
            metadata: BrowserResponseMetadata::default(),
        };

        let response = build_browser_response(&request, result);

        assert_eq!(response.url, "about:blank");
        assert_eq!(response.status, 0);
        assert!(response.headers.is_empty());
        assert_eq!(response.protocol.as_deref(), Some("browser"));
        assert!(response.ip_address.is_none());
        assert!(response.certificate.is_none());
    }

    #[test]
    fn browser_session_user_data_dir_is_stable_per_session_id() {
        let first = browser_session_user_data_dir("shared-browser");
        let second = browser_session_user_data_dir("shared-browser");
        let other = browser_session_user_data_dir("other-browser");

        assert_eq!(first, second);
        assert_ne!(first, other);
        assert!(
            first
                .to_string_lossy()
                .contains("halo-spider-playwright-sessions")
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
