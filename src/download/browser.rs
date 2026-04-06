use crate::download::traits::Downloader;
use crate::error::SpiderError;
#[cfg(any(feature = "browser", test))]
use crate::request::Headers;
use crate::request::browser::{Config as BrowserConfig, FingerprintProfile, SessionReuse};
use crate::request::{Request, RequestMode};
use crate::response::Response;
#[cfg(feature = "browser")]
use jiff::SignedDuration;
#[cfg(feature = "browser")]
use playwright_rs::protocol::{
    BrowserContext, BrowserContextOptions, ContinueOptions, Cookie, GotoOptions, Page, Playwright,
    ProxySettings, Viewport,
};
#[cfg(any(feature = "browser", test))]
use serde_json::json;
#[cfg(any(feature = "browser", test))]
use std::collections::BTreeMap;
#[cfg(any(feature = "browser", test))]
use std::path::PathBuf;
#[cfg(any(feature = "browser", test))]
use std::sync::Arc;
#[cfg(any(feature = "browser", test))]
use std::sync::OnceLock;
#[cfg(feature = "browser")]
use std::sync::atomic::AtomicBool;
#[cfg(any(feature = "browser", test))]
use std::sync::atomic::Ordering;
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
    request: &Request,
    config: &BrowserConfig,
) -> Result<(), SpiderError> {
    resolve_fingerprint_profile(config).map(|_| ())?;
    validate_browser_wait_for(config)?;
    validate_browser_session_reuse(request, config)?;

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

fn validate_browser_session_reuse(
    request: &Request,
    config: &BrowserConfig,
) -> Result<(), SpiderError> {
    if config.session_reuse != SessionReuse::Storage && request.session.is_none() {
        return Err(SpiderError::download(
            "browser session_reuse=context/page requires request.session",
        ));
    }

    Ok(())
}

fn validate_custom_fingerprint_profile(profile: &FingerprintProfile) -> Result<(), SpiderError> {
    validate_non_empty_browser_profile_field("user_agent", &profile.user_agent)?;
    validate_non_empty_browser_profile_field("locale", &profile.locale)?;
    validate_non_empty_browser_profile_field("timezone", &profile.timezone)?;
    validate_non_empty_browser_profile_field("accept_language", &profile.accept_language)?;
    validate_non_empty_browser_profile_field("platform", &profile.platform)?;
    validate_non_empty_browser_profile_field("vendor", &profile.vendor)?;

    if profile.languages.is_empty() {
        return Err(SpiderError::download(
            "browser custom_fingerprint_profile.languages must not be empty",
        ));
    }
    if profile
        .languages
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err(SpiderError::download(
            "browser custom_fingerprint_profile.languages must not contain empty values",
        ));
    }
    if profile.hardware_concurrency == 0 {
        return Err(SpiderError::download(
            "browser custom_fingerprint_profile.hardware_concurrency must be greater than 0",
        ));
    }
    if profile.device_memory == 0 {
        return Err(SpiderError::download(
            "browser custom_fingerprint_profile.device_memory must be greater than 0",
        ));
    }

    Ok(())
}

fn validate_non_empty_browser_profile_field(field: &str, value: &str) -> Result<(), SpiderError> {
    if value.trim().is_empty() {
        return Err(SpiderError::download(format!(
            "browser custom_fingerprint_profile.{field} must not be empty"
        )));
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BuiltinBrowserFingerprintProfile {
    name: &'static str,
    user_agent: &'static str,
    locale: &'static str,
    timezone: &'static str,
    accept_language: &'static str,
    languages: &'static [&'static str],
    platform: &'static str,
    vendor: &'static str,
    hardware_concurrency: u8,
    device_memory: u8,
    max_touch_points: u8,
}

impl BuiltinBrowserFingerprintProfile {
    fn to_profile(self) -> FingerprintProfile {
        FingerprintProfile::new()
            .with_user_agent(self.user_agent)
            .with_locale(self.locale)
            .with_timezone(self.timezone)
            .with_accept_language(self.accept_language)
            .with_languages(self.languages.iter().copied())
            .with_platform(self.platform)
            .with_vendor(self.vendor)
            .with_hardware_concurrency(self.hardware_concurrency)
            .with_device_memory(self.device_memory)
            .with_max_touch_points(self.max_touch_points)
    }
}

const BROWSER_FINGERPRINT_PROFILES: [BuiltinBrowserFingerprintProfile; 6] = [
    BuiltinBrowserFingerprintProfile {
        name: "desktop_zh_cn",
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
        locale: "zh-CN",
        timezone: "Asia/Shanghai",
        accept_language: "zh-CN,zh;q=0.9,en;q=0.8",
        languages: &["zh-CN", "zh", "en"],
        platform: "Win32",
        vendor: "Google Inc.",
        hardware_concurrency: 8,
        device_memory: 8,
        max_touch_points: 0,
    },
    BuiltinBrowserFingerprintProfile {
        name: "desktop_en_us",
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
        locale: "en-US",
        timezone: "America/New_York",
        accept_language: "en-US,en;q=0.9",
        languages: &["en-US", "en"],
        platform: "Win32",
        vendor: "Google Inc.",
        hardware_concurrency: 8,
        device_memory: 8,
        max_touch_points: 0,
    },
    BuiltinBrowserFingerprintProfile {
        name: "desktop_en_gb",
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
        locale: "en-GB",
        timezone: "Europe/London",
        accept_language: "en-GB,en;q=0.9",
        languages: &["en-GB", "en"],
        platform: "Win32",
        vendor: "Google Inc.",
        hardware_concurrency: 8,
        device_memory: 8,
        max_touch_points: 0,
    },
    BuiltinBrowserFingerprintProfile {
        name: "desktop_ja_jp",
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
        locale: "ja-JP",
        timezone: "Asia/Tokyo",
        accept_language: "ja-JP,ja;q=0.9,en-US;q=0.8,en;q=0.7",
        languages: &["ja-JP", "ja", "en-US", "en"],
        platform: "Win32",
        vendor: "Google Inc.",
        hardware_concurrency: 8,
        device_memory: 8,
        max_touch_points: 0,
    },
    BuiltinBrowserFingerprintProfile {
        name: "desktop_de_de",
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
        locale: "de-DE",
        timezone: "Europe/Berlin",
        accept_language: "de-DE,de;q=0.9,en;q=0.8",
        languages: &["de-DE", "de", "en"],
        platform: "Win32",
        vendor: "Google Inc.",
        hardware_concurrency: 8,
        device_memory: 8,
        max_touch_points: 0,
    },
    BuiltinBrowserFingerprintProfile {
        name: "desktop_fr_fr",
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
        locale: "fr-FR",
        timezone: "Europe/Paris",
        accept_language: "fr-FR,fr;q=0.9,en;q=0.8",
        languages: &["fr-FR", "fr", "en"],
        platform: "Win32",
        vendor: "Google Inc.",
        hardware_concurrency: 8,
        device_memory: 8,
        max_touch_points: 0,
    },
];

fn builtin_browser_fingerprint_profiles() -> &'static [BuiltinBrowserFingerprintProfile] {
    &BROWSER_FINGERPRINT_PROFILES
}

fn builtin_browser_fingerprint_profile_names() -> String {
    builtin_browser_fingerprint_profiles()
        .iter()
        .map(|profile| profile.name)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(any(feature = "browser", test))]
#[cfg_attr(all(test, not(feature = "browser")), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserExecutionPlan {
    profile: Option<FingerprintProfile>,
    init_script: Option<String>,
    session_reuse: SessionReuse,
}

#[cfg(any(feature = "browser", test))]
impl BrowserExecutionPlan {
    #[cfg_attr(all(test, not(feature = "browser")), allow(dead_code))]
    fn from_config(config: &BrowserConfig) -> Result<Self, SpiderError> {
        let profile = resolve_fingerprint_profile(config)?;
        let init_script = build_browser_init_script(config, profile.as_ref());

        Ok(Self {
            profile,
            init_script,
            session_reuse: config.session_reuse,
        })
    }
}

fn resolve_fingerprint_profile(
    config: &BrowserConfig,
) -> Result<Option<FingerprintProfile>, SpiderError> {
    if config.fingerprint_profile.is_some() && config.custom_fingerprint_profile.is_some() {
        return Err(SpiderError::download(
            "browser request cannot set both fingerprint_profile and custom_fingerprint_profile",
        ));
    }

    if let Some(profile) = config.custom_fingerprint_profile.as_ref() {
        validate_custom_fingerprint_profile(profile)?;
        return Ok(Some(profile.clone()));
    }

    let Some(profile_name) = config.fingerprint_profile.as_deref() else {
        return Ok(None);
    };

    if let Some(profile) = builtin_browser_fingerprint_profiles()
        .iter()
        .copied()
        .find(|profile| profile.name == profile_name)
    {
        return Ok(Some(profile.to_profile()));
    }

    let supported = builtin_browser_fingerprint_profile_names();
    Err(SpiderError::download(format!(
        "browser fingerprint_profile is not supported on the Playwright route: {profile_name}; supported profiles: {supported}"
    )))
}

#[cfg(any(feature = "browser", test))]
fn build_browser_init_script(
    config: &BrowserConfig,
    profile: Option<&FingerprintProfile>,
) -> Option<String> {
    if !config.stealth && profile.is_none() {
        return None;
    }

    let languages = profile
        .map(|profile| profile.languages.clone())
        .unwrap_or_else(|| vec!["en-US".to_string(), "en".to_string()]);
    let language = languages
        .first()
        .cloned()
        .unwrap_or_else(|| "en-US".to_string());
    let platform = profile
        .map(|profile| profile.platform.clone())
        .unwrap_or_else(|| "Win32".to_string());
    let vendor = profile
        .map(|profile| profile.vendor.clone())
        .unwrap_or_else(|| "Google Inc.".to_string());
    let hardware_concurrency = profile
        .map(|profile| profile.hardware_concurrency)
        .unwrap_or(8);
    let device_memory = profile.map(|profile| profile.device_memory).unwrap_or(8);
    let max_touch_points = profile.map(|profile| profile.max_touch_points).unwrap_or(0);
    let languages_json = json!(languages).to_string();
    let language_json = json!(language).to_string();
    let platform_json = json!(platform).to_string();
    let vendor_json = json!(vendor).to_string();
    let hardware_concurrency_json = json!(hardware_concurrency).to_string();
    let device_memory_json = json!(device_memory).to_string();
    let max_touch_points_json = json!(max_touch_points).to_string();
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
            "Object.defineProperty(navigator, 'language', {{ get: () => {language_json}, configurable: true }});"
        ));
        lines.push(format!(
            "Object.defineProperty(navigator, 'platform', {{ get: () => {platform_json}, configurable: true }});"
        ));
        lines.push(format!(
            "Object.defineProperty(navigator, 'vendor', {{ get: () => {vendor_json}, configurable: true }});"
        ));
        lines.push(format!(
            "Object.defineProperty(navigator, 'hardwareConcurrency', {{ get: () => {hardware_concurrency_json}, configurable: true }});"
        ));
        lines.push(format!(
            "Object.defineProperty(navigator, 'deviceMemory', {{ get: () => {device_memory_json}, configurable: true }});"
        ));
        lines.push(format!(
            "Object.defineProperty(navigator, 'maxTouchPoints', {{ get: () => {max_touch_points_json}, configurable: true }});"
        ));
    }

    if config.stealth {
        lines.push(
            "const haloPluginArray = [{ name: 'PDF Viewer', filename: 'internal-pdf-viewer', description: 'Portable Document Format' }];"
                .to_string(),
        );
        lines.push(
            "const haloMimeTypeArray = [{ type: 'application/pdf', suffixes: 'pdf', description: 'Portable Document Format' }];"
                .to_string(),
        );
        lines.push(
            "Object.defineProperty(navigator, 'plugins', { get: () => haloPluginArray.slice(), configurable: true });"
                .to_string(),
        );
        lines.push(
            "Object.defineProperty(navigator, 'mimeTypes', { get: () => haloMimeTypeArray.slice(), configurable: true });"
                .to_string(),
        );
        lines.push(
            "Object.defineProperty(navigator, 'pdfViewerEnabled', { get: () => true, configurable: true });"
                .to_string(),
        );
        lines.push(
            "Object.defineProperty(screen, 'colorDepth', { get: () => 24, configurable: true });"
                .to_string(),
        );
        lines.push(
            "Object.defineProperty(screen, 'pixelDepth', { get: () => 24, configurable: true });"
                .to_string(),
        );
        lines.push(
            "if (navigator.permissions && navigator.permissions.query) { const originalQuery = navigator.permissions.query.bind(navigator.permissions); navigator.permissions.query = (parameters) => { if (parameters && parameters.name === 'notifications') { return Promise.resolve({ state: Notification.permission }); } return originalQuery(parameters); }; }"
                .to_string(),
        );
        if config.engine == crate::request::browser::Engine::Chromium {
            lines.push(
                "if (!window.chrome) { Object.defineProperty(window, 'chrome', { value: {}, configurable: true }); }"
                    .to_string(),
            );
            lines.push(
                "if (!window.chrome.runtime) { Object.defineProperty(window.chrome, 'runtime', { value: {}, configurable: true }); }"
                    .to_string(),
            );
            lines.push(
                "if (!window.chrome.app) { Object.defineProperty(window.chrome, 'app', { value: { isInstalled: false }, configurable: true }); }"
                    .to_string(),
            );
            lines.push(format!(
                "Object.defineProperty(navigator, 'userAgentData', {{ get: () => ({{ brands: [{{ brand: 'Chromium', version: '136' }}, {{ brand: 'Not.A/Brand', version: '24' }}], mobile: false, platform: {platform_json}, getHighEntropyValues: async () => ({{ architecture: 'x86', bitness: '64', model: '', platform: {platform_json}, platformVersion: '10.0.0', uaFullVersion: '136.0.0.0' }}) }}), configurable: true }});"
            ));
        }
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
#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserSessionSignature {
    engine: crate::request::browser::Engine,
    headless: bool,
    viewport: crate::request::browser::Viewport,
    stealth: bool,
    session_reuse: SessionReuse,
    profile: Option<FingerprintProfile>,
    proxy: Option<String>,
}

#[cfg(feature = "browser")]
impl BrowserSessionSignature {
    fn for_request(
        request: &Request,
        config: &BrowserConfig,
        execution_plan: &BrowserExecutionPlan,
    ) -> Self {
        Self {
            engine: config.engine,
            headless: config.headless,
            viewport: config.viewport.clone(),
            stealth: config.stealth,
            session_reuse: execution_plan.session_reuse,
            profile: execution_plan.profile.clone(),
            proxy: request.proxy.as_ref().map(|proxy| proxy.url.clone()),
        }
    }
}

#[cfg(feature = "browser")]
struct BrowserLiveSession {
    signature: BrowserSessionSignature,
    #[allow(dead_code)]
    // Keeps the underlying Playwright server process alive for this live session.
    playwright: Playwright,
    context: BrowserContext,
    page: Option<Page>,
}

#[cfg(feature = "browser")]
async fn fetch_with_playwright_inner(
    request: &Request,
    config: &BrowserConfig,
) -> Result<BrowserFetchResult, SpiderError> {
    let _session_execution_guard = acquire_browser_session_execution_guard(request).await;
    let execution_plan = BrowserExecutionPlan::from_config(config)?;

    if request.session.is_some() && execution_plan.session_reuse != SessionReuse::Storage {
        return fetch_with_playwright_live_session(request, config, &execution_plan).await;
    }

    fetch_with_playwright_isolated_session(request, config, &execution_plan).await
}

#[cfg(feature = "browser")]
async fn fetch_with_playwright_isolated_session(
    request: &Request,
    config: &BrowserConfig,
    execution_plan: &BrowserExecutionPlan,
) -> Result<BrowserFetchResult, SpiderError> {
    let (playwright, context, user_data_dir) =
        launch_browser_context(request, config, execution_plan).await?;
    let outcome = run_browser_request_in_context(
        &context,
        request,
        config,
        execution_plan.profile.as_ref(),
        false,
        None,
    )
    .await
    .map(|(result, _page)| result);

    let close_outcome = context.close().await.map_err(map_playwright_error);
    let shutdown_outcome = playwright.shutdown().await.map_err(map_playwright_error);
    let cleanup_outcome = user_data_dir.cleanup().await;

    match outcome {
        Err(error) => {
            let _ = close_outcome;
            let _ = shutdown_outcome;
            cleanup_outcome?;
            Err(error)
        }
        Ok(result) => {
            close_outcome?;
            shutdown_outcome?;
            cleanup_outcome?;
            Ok(result)
        }
    }
}

#[cfg(feature = "browser")]
async fn fetch_with_playwright_live_session(
    request: &Request,
    config: &BrowserConfig,
    execution_plan: &BrowserExecutionPlan,
) -> Result<BrowserFetchResult, SpiderError> {
    let session_id = request
        .session
        .as_ref()
        .map(|session| session.id.clone())
        .ok_or_else(|| SpiderError::download("browser live session requires request.session"))?;
    let signature = BrowserSessionSignature::for_request(request, config, execution_plan);
    let mut session = match take_browser_live_session(&session_id).await {
        Some(session) => {
            if session.signature != signature {
                store_browser_live_session(&session_id, session).await;
                return Err(browser_live_session_mismatch_error(&session_id));
            }
            session
        }
        None => {
            let (playwright, context, _user_data_dir) =
                launch_browser_context(request, config, execution_plan).await?;
            BrowserLiveSession {
                signature: signature.clone(),
                playwright,
                context,
                page: None,
            }
        }
    };

    let keep_page = execution_plan.session_reuse == SessionReuse::Page;
    let existing_page = if keep_page { session.page.take() } else { None };
    let outcome = run_browser_request_in_context(
        &session.context,
        request,
        config,
        execution_plan.profile.as_ref(),
        keep_page,
        existing_page,
    )
    .await;

    match outcome {
        Ok((result, page)) => {
            session.page = page;
            store_browser_live_session(&session_id, session).await;
            Ok(result)
        }
        Err(error) => {
            session.page = None;
            store_browser_live_session(&session_id, session).await;
            Err(error)
        }
    }
}

#[cfg(feature = "browser")]
fn browser_live_session_mismatch_error(session_id: &str) -> SpiderError {
    SpiderError::download(format!(
        "browser live session `{session_id}` requires stable engine/headless/viewport/stealth/fingerprint_profile/proxy/session_reuse across requests"
    ))
}

#[cfg(feature = "browser")]
async fn launch_browser_context(
    request: &Request,
    config: &BrowserConfig,
    execution_plan: &BrowserExecutionPlan,
) -> Result<(Playwright, BrowserContext, BrowserUserDataDir), SpiderError> {
    let playwright = Playwright::launch().await.map_err(map_playwright_error)?;
    let options = build_context_options(
        config,
        request,
        request.timeout,
        execution_plan.profile.as_ref(),
    )?;
    let user_data_dir = BrowserUserDataDir::for_request(request).await?;
    let user_data_path = user_data_dir.path().to_string_lossy().into_owned();

    let context_result = match config.engine {
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
    };
    let context = match context_result {
        Ok(context) => context,
        Err(error) => {
            let cleanup_outcome = user_data_dir.cleanup().await;
            let shutdown_outcome = playwright.shutdown().await.map_err(map_playwright_error);
            cleanup_outcome?;
            shutdown_outcome?;
            return Err(map_playwright_error(error));
        }
    };

    if let Err(error) = apply_execution_plan_to_context(&context, execution_plan).await {
        let _ = context.close().await.map_err(map_playwright_error);
        let shutdown_outcome = playwright.shutdown().await.map_err(map_playwright_error);
        let cleanup_outcome = user_data_dir.cleanup().await;
        cleanup_outcome?;
        shutdown_outcome?;
        return Err(error);
    }

    Ok((playwright, context, user_data_dir))
}

#[cfg(feature = "browser")]
async fn run_browser_request_in_context(
    context: &BrowserContext,
    request: &Request,
    config: &BrowserConfig,
    profile: Option<&FingerprintProfile>,
    keep_page: bool,
    existing_page: Option<Page>,
) -> Result<(BrowserFetchResult, Option<Page>), SpiderError> {
    apply_request_state_to_context(context, request, profile).await?;

    let page = match existing_page.filter(|page| !page.is_closed()) {
        Some(page) => page,
        None => context.new_page().await.map_err(map_playwright_error)?,
    };
    let outcome = execute_browser_request_on_page(&page, request, config).await;

    match outcome {
        Ok(result) if keep_page && !page.is_closed() => Ok((result, Some(page))),
        Ok(result) => {
            if !page.is_closed() {
                page.close().await.map_err(map_playwright_error)?;
            }
            Ok((result, None))
        }
        Err(error) => {
            if !page.is_closed() {
                let _ = page.close().await.map_err(map_playwright_error);
            }
            Err(error)
        }
    }
}

#[cfg(feature = "browser")]
async fn apply_request_state_to_context(
    context: &BrowserContext,
    request: &Request,
    profile: Option<&FingerprintProfile>,
) -> Result<(), SpiderError> {
    context
        .set_extra_http_headers(build_browser_context_headers(request, profile))
        .await
        .map_err(map_playwright_error)?;
    apply_request_cookies_to_context(context, request).await
}

#[cfg(feature = "browser")]
async fn execute_browser_request_on_page(
    page: &Page,
    request: &Request,
    config: &BrowserConfig,
) -> Result<BrowserFetchResult, SpiderError> {
    let navigation_request_override = BrowserNavigationRequestOverride::for_request(request);

    if let Some(navigation_request_override) = navigation_request_override.clone() {
        let navigation_override_applied = Arc::new(AtomicBool::new(false));
        let navigation_override_state = Arc::clone(&navigation_override_applied);

        page.route("**", move |route| {
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

    let outcome = async {
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

    let cleanup_outcome = page.unroute_all(None).await.map_err(map_playwright_error);
    match outcome {
        Err(error) => {
            let _ = cleanup_outcome;
            Err(error)
        }
        Ok(result) => {
            cleanup_outcome?;
            Ok(result)
        }
    }
}

#[cfg(feature = "browser")]
fn build_context_options(
    config: &BrowserConfig,
    request: &Request,
    timeout: Option<SignedDuration>,
    profile: Option<&FingerprintProfile>,
) -> Result<BrowserContextOptions, SpiderError> {
    let mut builder = BrowserContextOptions::builder()
        .headless(config.headless)
        .viewport(Viewport {
            width: config.viewport.width,
            height: config.viewport.height,
        });

    if let Some(profile) = profile {
        builder = builder
            .user_agent(profile.user_agent.clone())
            .locale(profile.locale.clone())
            .timezone_id(profile.timezone.clone());
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
    context: &BrowserContext,
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
    profile: Option<&FingerprintProfile>,
) -> std::collections::HashMap<String, String> {
    let mut headers = BTreeMap::new();

    if let Some(profile) = profile {
        headers.insert(
            "accept-language".to_string(),
            profile.accept_language.clone(),
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
    context: &BrowserContext,
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
async fn browser_session_execution_lock(session_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static SESSION_LOCKS: OnceLock<
        tokio::sync::Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>,
    > = OnceLock::new();

    let locks = SESSION_LOCKS.get_or_init(|| tokio::sync::Mutex::new(BTreeMap::new()));
    let mut locks = locks.lock().await;

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

    let lock = browser_session_execution_lock(&session.id).await;
    let guard = lock.lock_owned().await;
    Some(BrowserSessionExecutionGuard { _guard: guard })
}

#[cfg(feature = "browser")]
fn browser_live_session_cache() -> &'static tokio::sync::Mutex<BTreeMap<String, BrowserLiveSession>>
{
    static LIVE_SESSIONS: OnceLock<tokio::sync::Mutex<BTreeMap<String, BrowserLiveSession>>> =
        OnceLock::new();

    LIVE_SESSIONS.get_or_init(|| tokio::sync::Mutex::new(BTreeMap::new()))
}

#[cfg(feature = "browser")]
async fn take_browser_live_session(session_id: &str) -> Option<BrowserLiveSession> {
    let cache = browser_live_session_cache();
    let mut cache = cache.lock().await;
    cache.remove(session_id)
}

#[cfg(feature = "browser")]
async fn store_browser_live_session(session_id: &str, session: BrowserLiveSession) {
    let cache = browser_live_session_cache();
    let mut cache = cache.lock().await;
    cache.insert(session_id.to_string(), session);
}

#[cfg(any(feature = "browser", test))]
#[cfg_attr(all(test, not(feature = "browser")), allow(dead_code))]
enum BrowserUserDataDir {
    Temporary(TemporaryUserDataDir),
    Persistent(PathBuf),
}

#[cfg(any(feature = "browser", test))]
#[cfg_attr(all(test, not(feature = "browser")), allow(dead_code))]
impl BrowserUserDataDir {
    async fn for_request(request: &Request) -> Result<Self, SpiderError> {
        let Some(session) = &request.session else {
            return Ok(Self::Temporary(TemporaryUserDataDir::new().await?));
        };

        let path = browser_session_user_data_dir(&session.id);
        tokio::fs::create_dir_all(&path).await.map_err(|error| {
            SpiderError::download(format!(
                "failed to create browser session user data dir: {error}"
            ))
        })?;

        Ok(Self::Persistent(path))
    }

    fn path(&self) -> &std::path::Path {
        match self {
            Self::Temporary(dir) => dir.path(),
            Self::Persistent(path) => path.as_path(),
        }
    }

    async fn cleanup(&self) -> Result<(), SpiderError> {
        match self {
            Self::Temporary(dir) => dir.cleanup().await,
            Self::Persistent(_) => Ok(()),
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

#[cfg(any(feature = "browser", test))]
struct TemporaryUserDataDir {
    path: std::path::PathBuf,
}

#[cfg(any(feature = "browser", test))]
impl TemporaryUserDataDir {
    async fn new() -> Result<Self, SpiderError> {
        static TEMP_DIR_COUNTER: OnceLock<std::sync::atomic::AtomicU64> = OnceLock::new();
        let counter = TEMP_DIR_COUNTER.get_or_init(|| std::sync::atomic::AtomicU64::new(1));
        let unique_id = counter.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "halo-spider-playwright-{}-{unique_id}",
            std::process::id()
        ));
        tokio::fs::create_dir_all(&path).await.map_err(|error| {
            SpiderError::download(format!(
                "failed to create browser temporary user data dir: {error}"
            ))
        })?;

        Ok(Self { path })
    }

    fn path(&self) -> &std::path::Path {
        self.path.as_path()
    }

    async fn cleanup(&self) -> Result<(), SpiderError> {
        match tokio::fs::remove_dir_all(&self.path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(SpiderError::download(format!(
                "failed to remove browser temporary user data dir: {error}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::traits::Downloader;
    use crate::request::browser::{Config as BrowserConfig, FingerprintProfile, SessionReuse};
    use std::sync::Arc;

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
    fn browser_request_contract_allows_custom_fingerprint_profile() {
        let request = Request::browser("https://example.com").with_browser(
            BrowserConfig::default().with_custom_fingerprint_profile(
                FingerprintProfile::new()
                    .with_locale("ja-JP")
                    .with_timezone("Asia/Tokyo")
                    .with_accept_language("ja-JP,ja;q=0.9")
                    .with_languages(["ja-JP", "ja"]),
            ),
        );

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
                "browser fingerprint_profile is not supported on the Playwright route: desktop_unknown; supported profiles: desktop_zh_cn, desktop_en_us, desktop_en_gb, desktop_ja_jp, desktop_de_de, desktop_fr_fr",
            )
        );
    }

    #[test]
    fn browser_request_contract_rejects_live_reuse_without_session() {
        let request = Request::browser("https://example.com")
            .with_browser(BrowserConfig::default().with_session_reuse(SessionReuse::Context));

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
            SpiderError::download("browser session_reuse=context/page requires request.session")
        );
    }

    #[test]
    fn browser_request_contract_allows_live_reuse_with_session() {
        let request = Request::browser("https://example.com")
            .with_session("shared-browser")
            .with_browser(BrowserConfig::default().with_session_reuse(SessionReuse::Page));

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

        assert_eq!(
            profile.user_agent,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36"
        );
        assert_eq!(profile.locale, "zh-CN");
        assert_eq!(profile.timezone, "Asia/Shanghai");
        assert_eq!(profile.languages, vec!["zh-CN", "zh", "en"]);
    }

    #[test]
    fn resolve_fingerprint_profile_returns_custom_profile() {
        let config = BrowserConfig::default().with_custom_fingerprint_profile(
            FingerprintProfile::new()
                .with_user_agent("custom-agent")
                .with_locale("fr-FR")
                .with_timezone("Europe/Paris")
                .with_accept_language("fr-FR,fr;q=0.9")
                .with_languages(["fr-FR", "fr"]),
        );

        let profile = resolve_fingerprint_profile(&config)
            .unwrap()
            .expect("profile should resolve");

        assert_eq!(profile.user_agent, "custom-agent");
        assert_eq!(profile.locale, "fr-FR");
        assert_eq!(profile.timezone, "Europe/Paris");
        assert_eq!(profile.languages, vec!["fr-FR", "fr"]);
    }

    #[test]
    fn resolve_fingerprint_profile_exposes_expanded_builtin_set() {
        let names = builtin_browser_fingerprint_profiles()
            .iter()
            .map(|profile| profile.name)
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "desktop_zh_cn",
                "desktop_en_us",
                "desktop_en_gb",
                "desktop_ja_jp",
                "desktop_de_de",
                "desktop_fr_fr",
            ]
        );
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
        assert!(init_script.contains("Object.defineProperty(navigator, 'language'"));
        assert!(init_script.contains("Object.defineProperty(navigator, 'hardwareConcurrency'"));
        assert!(init_script.contains("Object.defineProperty(navigator, 'plugins'"));
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
        let first = block_on(browser_session_execution_lock("shared-browser"));
        let second = block_on(browser_session_execution_lock("shared-browser"));
        let other = block_on(browser_session_execution_lock("other-browser"));

        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &other));
    }

    #[test]
    fn temporary_user_data_dir_is_created_and_cleaned_asynchronously() {
        let dir = block_on(TemporaryUserDataDir::new()).expect("temp dir should create");
        let path = dir.path().to_path_buf();

        assert!(path.exists());

        block_on(dir.cleanup()).expect("temp dir should clean");
        assert!(!path.exists());
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
                "browser fingerprint_profile is not supported on the Playwright route: desktop_unknown; supported profiles: desktop_zh_cn, desktop_en_us, desktop_en_gb, desktop_ja_jp, desktop_de_de, desktop_fr_fr",
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

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build")
            .block_on(future)
    }
}
