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
    BrowserContextOptions, ContinueOptions, Cookie, GotoOptions, Playwright, ProxySettings,
    Viewport,
};
#[cfg(any(feature = "browser", test))]
use serde_json::json;
#[cfg(any(feature = "browser", test))]
use std::collections::BTreeMap;
#[cfg(any(feature = "browser", test))]
use std::path::PathBuf;
#[cfg(any(feature = "browser", test))]
use std::sync::Arc;
#[cfg(feature = "browser")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(any(feature = "browser", test))]
use std::sync::{Mutex, OnceLock};
#[cfg(any(feature = "browser", test))]
use url::Url;

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
    resolve_fingerprint_profile(config).map(|_| ())?;
    validate_browser_wait_for(config)?;

    Ok(())
}

fn validate_browser_wait_for(config: &BrowserConfig) -> Result<(), SpiderError> {
    if let Some(selector) = &config.wait_for
        && selector.trim().is_empty()
    {
        return Err(SpiderError::download(
            "browser wait_for requires a non-empty selector",
        ));
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BrowserFingerprintProfile {
    name: &'static str,
    user_agent: &'static str,
    locale: &'static str,
    timezone_id: &'static str,
    accept_language: &'static str,
    languages: &'static [&'static str],
    platform: &'static str,
    vendor: &'static str,
}

#[cfg(feature = "browser")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserExecutionPlan {
    profile: Option<BrowserFingerprintProfile>,
    init_script: Option<String>,
}

#[cfg(feature = "browser")]
impl BrowserExecutionPlan {
    fn from_config(config: &BrowserConfig) -> Result<Self, SpiderError> {
        let profile = resolve_fingerprint_profile(config)?;
        let init_script = build_browser_init_script(config, profile.as_ref());

        Ok(Self {
            profile,
            init_script,
        })
    }
}

fn resolve_fingerprint_profile(
    config: &BrowserConfig,
) -> Result<Option<BrowserFingerprintProfile>, SpiderError> {
    let Some(profile_name) = config.fingerprint_profile.as_deref() else {
        return Ok(None);
    };

    match profile_name {
        "desktop_zh_cn" => Ok(Some(BrowserFingerprintProfile {
            name: "desktop_zh_cn",
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
            locale: "zh-CN",
            timezone_id: "Asia/Shanghai",
            accept_language: "zh-CN,zh;q=0.9,en;q=0.8",
            languages: &["zh-CN", "zh", "en"],
            platform: "Win32",
            vendor: "Google Inc.",
        })),
        "desktop_en_us" => Ok(Some(BrowserFingerprintProfile {
            name: "desktop_en_us",
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
            locale: "en-US",
            timezone_id: "America/New_York",
            accept_language: "en-US,en;q=0.9",
            languages: &["en-US", "en"],
            platform: "Win32",
            vendor: "Google Inc.",
        })),
        other => Err(SpiderError::download(format!(
            "browser fingerprint_profile is not supported on the Playwright route: {other}"
        ))),
    }
}

#[cfg(any(feature = "browser", test))]
fn build_browser_init_script(
    config: &BrowserConfig,
    profile: Option<&BrowserFingerprintProfile>,
) -> Option<String> {
    if !config.stealth && profile.is_none() {
        return None;
    }

    let languages = profile
        .map(|profile| profile.languages)
        .unwrap_or(&["en-US", "en"]);
    let platform = profile.map(|profile| profile.platform).unwrap_or("Win32");
    let vendor = profile
        .map(|profile| profile.vendor)
        .unwrap_or("Google Inc.");
    let languages_json = json!(languages).to_string();
    let platform_json = json!(platform).to_string();
    let vendor_json = json!(vendor).to_string();
    let mut lines = Vec::new();

    if config.stealth {
        lines.push(
            "Object.defineProperty(navigator, 'webdriver', { get: () => undefined, configurable: true });"
                .to_string(),
        );
    }

    if config.stealth || profile.is_some() {
        lines.push(format!(
            "Object.defineProperty(navigator, 'languages', {{ get: () => {languages_json}.slice(), configurable: true }});"
        ));
        lines.push(format!(
            "Object.defineProperty(navigator, 'platform', {{ get: () => {platform_json}, configurable: true }});"
        ));
        lines.push(format!(
            "Object.defineProperty(navigator, 'vendor', {{ get: () => {vendor_json}, configurable: true }});"
        ));
        lines.push(
            "if (!window.chrome) { Object.defineProperty(window, 'chrome', { value: { runtime: {} }, configurable: true }); }"
                .to_string(),
        );
    }

    if config.stealth {
        lines.push(
            "if (navigator.permissions && navigator.permissions.query) { const originalQuery = navigator.permissions.query.bind(navigator.permissions); navigator.permissions.query = (parameters) => { if (parameters && parameters.name === 'notifications') { return Promise.resolve({ state: Notification.permission }); } return originalQuery(parameters); }; }"
                .to_string(),
        );
    }

    Some(format!("(() => {{\n{}\n}})();", lines.join("\n")))
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
    let _session_execution_guard = acquire_browser_session_execution_guard(request).await;
    let playwright = Playwright::launch().await.map_err(map_playwright_error)?;
    let execution_plan = BrowserExecutionPlan::from_config(config)?;
    let options = build_context_options(
        config,
        request,
        request.timeout,
        execution_plan.profile.as_ref(),
    )?;
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
        apply_execution_plan_to_context(&context, &execution_plan).await?;
        apply_request_cookies_to_context(&context, request).await?;

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

        let frame = page.main_frame().await.map_err(map_playwright_error)?;

        if let Some(selector) = &config.wait_for {
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
    profile: Option<&BrowserFingerprintProfile>,
) -> Result<BrowserContextOptions, SpiderError> {
    let mut builder = BrowserContextOptions::builder()
        .headless(config.headless)
        .viewport(Viewport {
            width: config.viewport.width,
            height: config.viewport.height,
        });

    if let Some(profile) = profile {
        builder = builder
            .user_agent(profile.user_agent.to_string())
            .locale(profile.locale.to_string())
            .timezone_id(profile.timezone_id.to_string());
    }

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

    let headers = build_browser_context_headers(request, profile);
    if !headers.is_empty() {
        builder = builder.extra_http_headers(headers);
    }

    Ok(builder.build())
}

#[cfg(feature = "browser")]
async fn apply_execution_plan_to_context(
    context: &playwright_rs::protocol::BrowserContext,
    execution_plan: &BrowserExecutionPlan,
) -> Result<(), SpiderError> {
    let Some(init_script) = execution_plan.init_script.as_deref() else {
        return Ok(());
    };

    context
        .add_init_script(init_script)
        .await
        .map_err(map_playwright_error)
}

#[cfg(any(feature = "browser", test))]
fn build_browser_context_headers(
    request: &Request,
    profile: Option<&BrowserFingerprintProfile>,
) -> std::collections::HashMap<String, String> {
    let mut headers = BTreeMap::new();

    if let Some(profile) = profile {
        headers.insert(
            "accept-language".to_string(),
            profile.accept_language.to_string(),
        );
    }

    for (key, values) in &request.headers {
        if let Some(existing_key) = headers
            .keys()
            .find(|existing| existing.eq_ignore_ascii_case(key))
            .cloned()
        {
            headers.remove(&existing_key);
        }
        headers.insert(key.clone(), values.join(", "));
    }

    headers.into_iter().collect()
}

#[cfg(any(feature = "browser", test))]
fn build_browser_request_cookies(
    request: &Request,
) -> Result<Vec<BrowserRequestCookie>, SpiderError> {
    if request.cookies.is_empty() {
        return Ok(Vec::new());
    }

    let url =
        Url::parse(&request.url).map_err(|error| SpiderError::request_build(error.to_string()))?;
    let host = url.host_str().ok_or_else(|| {
        SpiderError::request_build("browser request cookies require a URL host".to_string())
    })?;
    let secure = url.scheme().eq_ignore_ascii_case("https");

    Ok(request
        .cookies
        .iter()
        .map(|(name, value)| BrowserRequestCookie {
            name: name.clone(),
            value: value.clone(),
            domain: host.to_string(),
            path: "/".to_string(),
            expires: -1.0,
            http_only: false,
            secure,
            same_site: None,
        })
        .collect())
}

#[cfg(feature = "browser")]
async fn apply_request_cookies_to_context(
    context: &playwright_rs::protocol::BrowserContext,
    request: &Request,
) -> Result<(), SpiderError> {
    let cookies = build_browser_request_cookies(request)?;
    if cookies.is_empty() {
        return Ok(());
    }

    context
        .add_cookies(&cookies)
        .await
        .map_err(map_playwright_error)
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
type BrowserRequestCookie = Cookie;

#[cfg(all(test, not(feature = "browser")))]
#[derive(Debug, Clone, PartialEq)]
struct BrowserRequestCookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    expires: f64,
    http_only: bool,
    secure: bool,
    same_site: Option<String>,
}

#[cfg(any(feature = "browser", test))]
fn browser_session_execution_lock(session_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static SESSION_LOCKS: OnceLock<Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();

    let locks = SESSION_LOCKS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut locks = locks.lock().expect("browser session lock map poisoned");

    locks
        .entry(session_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

#[cfg(feature = "browser")]
struct BrowserSessionExecutionGuard {
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

#[cfg(feature = "browser")]
async fn acquire_browser_session_execution_guard(
    request: &Request,
) -> Option<BrowserSessionExecutionGuard> {
    let Some(session) = &request.session else {
        return None;
    };

    let lock = browser_session_execution_lock(&session.id);
    let guard = lock.lock_owned().await;
    Some(BrowserSessionExecutionGuard { _guard: guard })
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
    fn browser_request_contract_allows_stealth() {
        let request = Request::browser("https://example.com")
            .with_browser(BrowserConfig::default().with_stealth(true));

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
    fn browser_request_contract_allows_supported_fingerprint_profile() {
        let request = Request::browser("https://example.com")
            .with_browser(BrowserConfig::default().with_fingerprint_profile("desktop_zh_cn"));

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
    fn browser_request_contract_rejects_unsupported_fingerprint_profile() {
        let request = Request::browser("https://example.com")
            .with_browser(BrowserConfig::default().with_fingerprint_profile("desktop_unknown"));

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
                "browser fingerprint_profile is not supported on the Playwright route: desktop_unknown",
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
    fn browser_request_contract_allows_wait_for_selector() {
        let request = Request::browser("https://example.com")
            .with_browser(BrowserConfig::default().with_wait_for(".result"));

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
    fn browser_request_contract_rejects_empty_wait_for_selector() {
        let request = Request::browser("https://example.com")
            .with_browser(BrowserConfig::default().with_wait_for("   "));

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
            SpiderError::download("browser wait_for requires a non-empty selector")
        );
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
    fn build_browser_request_cookies_uses_request_url_host() {
        let request = Request::browser("https://example.com/detail").with_cookie("sid", "abc");

        let cookies = build_browser_request_cookies(&request).expect("cookies should build");

        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "sid");
        assert_eq!(cookies[0].value, "abc");
        assert_eq!(cookies[0].domain, "example.com");
        assert_eq!(cookies[0].path, "/");
        assert!(cookies[0].secure);
    }

    #[test]
    fn resolve_fingerprint_profile_returns_builtin_profile() {
        let config = BrowserConfig::default().with_fingerprint_profile("desktop_zh_cn");

        let profile = resolve_fingerprint_profile(&config)
            .unwrap()
            .expect("profile should resolve");

        assert_eq!(profile.name, "desktop_zh_cn");
        assert_eq!(profile.locale, "zh-CN");
        assert_eq!(profile.timezone_id, "Asia/Shanghai");
        assert_eq!(profile.languages, ["zh-CN", "zh", "en"]);
    }

    #[test]
    fn build_browser_init_script_supports_profile_and_stealth() {
        let config = BrowserConfig::default()
            .with_stealth(true)
            .with_fingerprint_profile("desktop_en_us");
        let profile = resolve_fingerprint_profile(&config)
            .unwrap()
            .expect("profile should resolve");
        let init_script =
            build_browser_init_script(&config, Some(&profile)).expect("init script should exist");

        assert!(init_script.contains("Object.defineProperty(navigator, 'webdriver'"));
        assert!(init_script.contains("Object.defineProperty(navigator, 'languages'"));
        assert!(init_script.contains("navigator.permissions.query"));
        assert!(init_script.contains("en-US"));
        assert!(init_script.contains("Win32"));
    }

    #[test]
    fn build_browser_context_headers_prefers_request_header_over_profile_default() {
        let request = Request::browser("https://example.com")
            .with_header("Accept-Language", "fr-FR,fr;q=0.9")
            .with_header("x-token", "abc");
        let profile = resolve_fingerprint_profile(
            &BrowserConfig::default().with_fingerprint_profile("desktop_zh_cn"),
        )
        .unwrap()
        .expect("profile should resolve");

        let headers = build_browser_context_headers(&request, Some(&profile));

        assert_eq!(
            headers.get("Accept-Language").map(String::as_str),
            Some("fr-FR,fr;q=0.9")
        );
        assert!(!headers.contains_key("accept-language"));
        assert_eq!(headers.get("x-token").map(String::as_str), Some("abc"));
    }

    #[test]
    fn browser_session_execution_lock_is_stable_per_session_id() {
        let first = browser_session_execution_lock("shared-browser");
        let second = browser_session_execution_lock("shared-browser");
        let other = browser_session_execution_lock("other-browser");

        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &other));
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
            .with_browser(BrowserConfig::default().with_fingerprint_profile("desktop_unknown"));

        let error = block_on(downloader.fetch(&request)).unwrap_err();

        assert_eq!(
            error,
            SpiderError::download(
                "browser fingerprint_profile is not supported on the Playwright route: desktop_unknown",
            )
        );
    }

    #[cfg(feature = "browser")]
    #[test]
    fn build_context_options_matches_browser_contract() {
        let config = BrowserConfig::default()
            .with_headless(false)
            .with_viewport(1440, 900)
            .with_fingerprint_profile("desktop_zh_cn");
        let request = Request::browser("https://example.com")
            .with_header("Accept-Language", "fr-FR,fr;q=0.9")
            .with_header("x-token", "abc")
            .with_proxy("http://127.0.0.1:8080");
        let profile = resolve_fingerprint_profile(&config)
            .unwrap()
            .expect("profile should resolve");
        let options = build_context_options(
            &config,
            &request,
            Some(SignedDuration::from_secs(8)),
            Some(&profile),
        )
        .unwrap();

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
        assert_eq!(options.user_agent.as_deref(), Some(profile.user_agent));
        assert_eq!(options.locale.as_deref(), Some("zh-CN"));
        assert_eq!(options.timezone_id.as_deref(), Some("Asia/Shanghai"));
        assert_eq!(
            options
                .extra_http_headers
                .as_ref()
                .and_then(|headers| headers.get("Accept-Language"))
                .map(String::as_str),
            Some("fr-FR,fr;q=0.9")
        );
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
