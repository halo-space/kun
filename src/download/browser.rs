use crate::download::traits::Downloader;
use crate::error::SpiderError;
#[cfg(any(feature = "browser", test))]
use crate::request::Headers;
#[cfg(any(feature = "browser", test))]
use crate::request::browser::KeepAliveOnError;
#[cfg(any(feature = "browser", test))]
use crate::request::browser::KeepAliveScope;
use crate::request::browser::{
    Config as BrowserConfig, Engine as BrowserEngine, FingerprintProfile, KeepAlive, ScreenProfile,
    Size,
};
use crate::request::{Request, RequestMode};
use crate::response::Response;
use jiff::SignedDuration;
#[cfg(feature = "browser")]
use playwright_rs::protocol::{
    BrowserContext, BrowserContextOptions, ContinueOptions, Cookie, GotoOptions, Page, Playwright,
    ProxySettings, Viewport,
};
use serde::{Deserialize, Serialize};
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
use std::time::Instant;
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
    resolve_device_profile(config).map(|_| ())?;
    validate_browser_stealth_scripts(config)?;
    validate_browser_wait_for_selector(config)?;
    validate_browser_keep_alive(request, config)?;

    Ok(())
}

fn validate_browser_stealth_scripts(config: &BrowserConfig) -> Result<(), SpiderError> {
    if config
        .stealth_scripts
        .iter()
        .any(|script| script.trim().is_empty())
    {
        return Err(SpiderError::download(
            "browser stealth_scripts must not contain empty scripts",
        ));
    }

    Ok(())
}

fn validate_browser_wait_for_selector(config: &BrowserConfig) -> Result<(), SpiderError> {
    if let Some(selector) = &config.wait_for_selector
        && selector.trim().is_empty()
    {
        return Err(SpiderError::download(
            "browser wait_for_selector requires a non-empty selector",
        ));
    }

    Ok(())
}

fn validate_browser_keep_alive(
    request: &Request,
    config: &BrowserConfig,
) -> Result<(), SpiderError> {
    if let Some(max_idle) = config.keep_alive_max_idle
        && max_idle <= SignedDuration::ZERO
    {
        return Err(SpiderError::download(
            "browser keep_alive_max_idle must be greater than 0",
        ));
    }

    if config.keep_alive != KeepAlive::Isolated && request.session.is_none() {
        return Err(SpiderError::download(
            "browser keep_alive=context/page requires request.session",
        ));
    }

    Ok(())
}

fn validate_fingerprint_profile(profile: &FingerprintProfile) -> Result<(), SpiderError> {
    validate_optional_non_empty_browser_profile_field("user_agent", profile.user_agent.as_deref())?;
    validate_optional_non_empty_browser_profile_field("locale", profile.locale.as_deref())?;
    validate_optional_non_empty_browser_profile_field("timezone", profile.timezone.as_deref())?;
    validate_optional_non_empty_browser_profile_field(
        "accept_language",
        profile.accept_language.as_deref(),
    )?;
    validate_optional_non_empty_browser_profile_field("platform", profile.platform.as_deref())?;

    if let Some(languages) = profile.languages.as_ref() {
        if languages.is_empty() {
            return Err(SpiderError::download(
                "browser device_profile.fingerprint.languages must not be empty",
            ));
        }
        if languages.iter().any(|value| value.trim().is_empty()) {
            return Err(SpiderError::download(
                "browser device_profile.fingerprint.languages must not contain empty values",
            ));
        }
    }
    if profile.device_memory == Some(0) {
        return Err(SpiderError::download(
            "browser device_profile.fingerprint.device_memory must be greater than 0",
        ));
    }

    Ok(())
}

fn validate_screen_profile(profile: &ScreenProfile) -> Result<(), SpiderError> {
    validate_browser_screen_size("viewport", profile.viewport.as_ref())?;
    validate_browser_screen_size("screen", profile.screen.as_ref())?;
    validate_browser_screen_size("avail", profile.avail.as_ref())?;

    if profile.color_depth == Some(0) {
        return Err(SpiderError::download(
            "browser device_profile.screen.color_depth must be greater than 0",
        ));
    }
    if profile.pixel_depth == Some(0) {
        return Err(SpiderError::download(
            "browser device_profile.screen.pixel_depth must be greater than 0",
        ));
    }
    if profile.device_scale_factor == Some(0) {
        return Err(SpiderError::download(
            "browser device_profile.screen.device_scale_factor must be greater than 0",
        ));
    }

    Ok(())
}

fn validate_browser_screen_size(field: &str, value: Option<&Size>) -> Result<(), SpiderError> {
    let Some(value) = value else {
        return Ok(());
    };

    if value.width == 0 || value.height == 0 {
        return Err(SpiderError::download(format!(
            "browser device_profile.screen.{field} width and height must be greater than 0"
        )));
    }

    Ok(())
}

fn validate_optional_non_empty_browser_profile_field(
    field: &str,
    value: Option<&str>,
) -> Result<(), SpiderError> {
    if let Some(value) = value
        && value.trim().is_empty()
    {
        return Err(SpiderError::download(format!(
            "browser device_profile.fingerprint.{field} must not be empty"
        )));
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BuiltinBrowserFingerprintDefaults {
    user_agent: &'static str,
    locale: &'static str,
    timezone: &'static str,
    accept_language: &'static str,
    languages: &'static [&'static str],
    platform: &'static str,
    mobile: bool,
    vendor: &'static str,
    hardware_concurrency: u8,
    device_memory: u8,
    max_touch_points: u8,
}

fn builtin_browser_fingerprint_defaults(
    engine: BrowserEngine,
    mobile: bool,
) -> BuiltinBrowserFingerprintDefaults {
    match (engine, mobile) {
        (BrowserEngine::Chromium, false) => BuiltinBrowserFingerprintDefaults {
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36",
            locale: "en-US",
            timezone: "America/New_York",
            accept_language: "en-US,en;q=0.9",
            languages: &["en-US", "en"],
            platform: "Win32",
            mobile: false,
            vendor: "Google Inc.",
            hardware_concurrency: 8,
            device_memory: 8,
            max_touch_points: 0,
        },
        (BrowserEngine::Chromium, true) => BuiltinBrowserFingerprintDefaults {
            user_agent: "Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Mobile Safari/537.36",
            locale: "en-US",
            timezone: "America/New_York",
            accept_language: "en-US,en;q=0.9",
            languages: &["en-US", "en"],
            platform: "Linux armv81",
            mobile: true,
            vendor: "Google Inc.",
            hardware_concurrency: 8,
            device_memory: 8,
            max_touch_points: 5,
        },
        (BrowserEngine::Firefox, false) => BuiltinBrowserFingerprintDefaults {
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:137.0) Gecko/20100101 Firefox/137.0",
            locale: "en-US",
            timezone: "America/New_York",
            accept_language: "en-US,en;q=0.9",
            languages: &["en-US", "en"],
            platform: "Win32",
            mobile: false,
            vendor: "",
            hardware_concurrency: 8,
            device_memory: 8,
            max_touch_points: 0,
        },
        (BrowserEngine::Firefox, true) => BuiltinBrowserFingerprintDefaults {
            user_agent: "Mozilla/5.0 (Android 14; Mobile; rv:137.0) Gecko/137.0 Firefox/137.0",
            locale: "en-US",
            timezone: "America/New_York",
            accept_language: "en-US,en;q=0.9",
            languages: &["en-US", "en"],
            platform: "Linux armv81",
            mobile: true,
            vendor: "",
            hardware_concurrency: 8,
            device_memory: 8,
            max_touch_points: 5,
        },
        (BrowserEngine::Webkit, false) => BuiltinBrowserFingerprintDefaults {
            user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_4) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15",
            locale: "en-US",
            timezone: "America/New_York",
            accept_language: "en-US,en;q=0.9",
            languages: &["en-US", "en"],
            platform: "MacIntel",
            mobile: false,
            vendor: "Apple Computer, Inc.",
            hardware_concurrency: 8,
            device_memory: 8,
            max_touch_points: 0,
        },
        (BrowserEngine::Webkit, true) => BuiltinBrowserFingerprintDefaults {
            user_agent: "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1",
            locale: "en-US",
            timezone: "America/New_York",
            accept_language: "en-US,en;q=0.9",
            languages: &["en-US", "en"],
            platform: "iPhone",
            mobile: true,
            vendor: "Apple Computer, Inc.",
            hardware_concurrency: 8,
            device_memory: 4,
            max_touch_points: 5,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BrowserResolvedFingerprintProfile {
    user_agent: String,
    locale: String,
    timezone: String,
    accept_language: String,
    languages: Vec<String>,
    platform: String,
    mobile: bool,
    vendor: String,
    hardware_concurrency: u8,
    device_memory: u8,
    max_touch_points: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BrowserResolvedScreenProfile {
    viewport: Size,
    screen: Size,
    avail: Size,
    color_depth: u8,
    pixel_depth: u8,
    device_scale_factor: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BrowserResolvedDeviceProfile {
    fingerprint: BrowserResolvedFingerprintProfile,
    screen: BrowserResolvedScreenProfile,
}

#[cfg(any(feature = "browser", test))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BrowserSessionDeviceProfileState {
    engine: crate::request::browser::Engine,
    device_profile: BrowserResolvedDeviceProfile,
}

fn default_browser_screen_profile(mobile: bool) -> BrowserResolvedScreenProfile {
    let default = if mobile {
        Size::new(390, 844)
    } else {
        Size::new(1280, 720)
    };
    BrowserResolvedScreenProfile {
        viewport: default.clone(),
        screen: default.clone(),
        avail: default,
        color_depth: 24,
        pixel_depth: 24,
        device_scale_factor: if mobile { 3 } else { 1 },
    }
}

#[allow(dead_code)]
fn browser_execution_viewport(device_profile: Option<&BrowserResolvedDeviceProfile>) -> Size {
    device_profile
        .map(|profile| profile.screen.viewport.clone())
        .unwrap_or_else(|| default_browser_screen_profile(false).viewport)
}

fn max_browser_size(left: &Size, right: &Size) -> Size {
    Size::new(left.width.max(right.width), left.height.max(right.height))
}

fn resolve_screen_profile(
    profile: Option<&ScreenProfile>,
    mobile: bool,
) -> Result<BrowserResolvedScreenProfile, SpiderError> {
    if let Some(profile) = profile {
        validate_screen_profile(profile)?;
    }

    let default = default_browser_screen_profile(mobile);
    let input_viewport = profile.and_then(|screen| screen.viewport.clone());
    let input_screen = profile.and_then(|screen| screen.screen.clone());
    let input_avail = profile.and_then(|screen| screen.avail.clone());

    let viewport = input_viewport
        .clone()
        .or_else(|| input_screen.clone())
        .or_else(|| input_avail.clone())
        .unwrap_or_else(|| default.viewport.clone());
    let screen = match (input_screen, input_avail.clone()) {
        (Some(screen), _) => screen,
        (None, Some(avail)) => max_browser_size(&viewport, &avail),
        (None, None) => viewport.clone(),
    };
    let avail = input_avail.unwrap_or_else(|| screen.clone());

    if screen.width < viewport.width || screen.height < viewport.height {
        return Err(SpiderError::download(
            "browser device_profile.screen requires screen >= viewport",
        ));
    }
    if screen.width < avail.width || screen.height < avail.height {
        return Err(SpiderError::download(
            "browser device_profile.screen requires screen >= avail",
        ));
    }

    Ok(BrowserResolvedScreenProfile {
        viewport,
        screen,
        avail,
        color_depth: profile
            .and_then(|screen| screen.color_depth)
            .unwrap_or(default.color_depth),
        pixel_depth: profile
            .and_then(|screen| screen.pixel_depth)
            .unwrap_or(default.pixel_depth),
        device_scale_factor: profile
            .and_then(|screen| screen.device_scale_factor)
            .unwrap_or(default.device_scale_factor),
    })
}

fn resolve_fingerprint_profile(
    profile: Option<&FingerprintProfile>,
    engine: BrowserEngine,
) -> Result<BrowserResolvedFingerprintProfile, SpiderError> {
    if let Some(profile) = profile {
        validate_fingerprint_profile(profile)?;
    }

    let requested_mobile = profile.and_then(|profile| profile.mobile).unwrap_or(false);
    let defaults = builtin_browser_fingerprint_defaults(engine, requested_mobile);

    Ok(BrowserResolvedFingerprintProfile {
        user_agent: profile
            .and_then(|profile| profile.user_agent.clone())
            .unwrap_or_else(|| defaults.user_agent.to_string()),
        locale: profile
            .and_then(|profile| profile.locale.clone())
            .unwrap_or_else(|| defaults.locale.to_string()),
        timezone: profile
            .and_then(|profile| profile.timezone.clone())
            .unwrap_or_else(|| defaults.timezone.to_string()),
        accept_language: profile
            .and_then(|profile| profile.accept_language.clone())
            .unwrap_or_else(|| defaults.accept_language.to_string()),
        languages: profile
            .and_then(|profile| profile.languages.clone())
            .unwrap_or_else(|| {
                defaults
                    .languages
                    .iter()
                    .map(|value| value.to_string())
                    .collect()
            }),
        platform: profile
            .and_then(|profile| profile.platform.clone())
            .unwrap_or_else(|| defaults.platform.to_string()),
        mobile: profile
            .and_then(|profile| profile.mobile)
            .unwrap_or(defaults.mobile),
        vendor: defaults.vendor.to_string(),
        hardware_concurrency: defaults.hardware_concurrency,
        device_memory: profile
            .and_then(|profile| profile.device_memory)
            .unwrap_or(defaults.device_memory),
        max_touch_points: defaults.max_touch_points,
    })
}

fn resolve_device_profile(
    config: &BrowserConfig,
) -> Result<Option<BrowserResolvedDeviceProfile>, SpiderError> {
    if let Some(device_profile) = config.device_profile.as_ref() {
        let fingerprint =
            resolve_fingerprint_profile(device_profile.fingerprint.as_ref(), config.engine)?;
        return Ok(Some(BrowserResolvedDeviceProfile {
            screen: resolve_screen_profile(device_profile.screen.as_ref(), fingerprint.mobile)?,
            fingerprint,
        }));
    }

    if config.stealth {
        return Ok(Some(BrowserResolvedDeviceProfile::default_for_stealth(
            config.engine,
        )));
    }

    Ok(None)
}

impl BrowserResolvedDeviceProfile {
    #[allow(dead_code)]
    fn default_for_stealth(engine: BrowserEngine) -> Self {
        let fingerprint = resolve_fingerprint_profile(None, engine)
            .expect("builtin browser fingerprint defaults must be valid");
        Self {
            screen: default_browser_screen_profile(fingerprint.mobile),
            fingerprint,
        }
    }
}

#[cfg(any(feature = "browser", test))]
#[cfg_attr(all(test, not(feature = "browser")), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserExecutionPlan {
    device_profile: Option<BrowserResolvedDeviceProfile>,
    init_script: Option<String>,
    keep_alive: KeepAlive,
    keep_alive_scope: KeepAliveScope,
    keep_alive_key: Option<String>,
    keep_alive_max_idle: Option<SignedDuration>,
    keep_alive_max_uses: Option<u64>,
    keep_alive_on_error: KeepAliveOnError,
}

#[cfg(any(feature = "browser", test))]
impl BrowserExecutionPlan {
    #[cfg_attr(all(test, not(feature = "browser")), allow(dead_code))]
    async fn from_request(request: &Request, config: &BrowserConfig) -> Result<Self, SpiderError> {
        let device_profile = resolve_effective_device_profile(request, config).await?;
        let init_script = build_browser_init_script(config, device_profile.as_ref());

        Ok(Self {
            device_profile,
            init_script,
            keep_alive: config.keep_alive,
            keep_alive_scope: config.keep_alive_scope,
            keep_alive_key: config.keep_alive_key.clone(),
            keep_alive_max_idle: config.keep_alive_max_idle,
            keep_alive_max_uses: config.keep_alive_max_uses,
            keep_alive_on_error: config.keep_alive_on_error,
        })
    }
}

#[cfg(any(feature = "browser", test))]
fn build_browser_init_script(
    config: &BrowserConfig,
    device_profile: Option<&BrowserResolvedDeviceProfile>,
) -> Option<String> {
    if !config.stealth && device_profile.is_none() && config.stealth_scripts.is_empty() {
        return None;
    }

    let fallback = BrowserResolvedDeviceProfile::default_for_stealth(config.engine);
    let fingerprint = device_profile
        .map(|profile| &profile.fingerprint)
        .unwrap_or(&fallback.fingerprint);
    let screen = device_profile
        .map(|profile| &profile.screen)
        .unwrap_or(&fallback.screen);
    let language = fingerprint
        .languages
        .first()
        .cloned()
        .unwrap_or_else(|| "en-US".to_string());
    let languages_json = json!(fingerprint.languages).to_string();
    let language_json = json!(language).to_string();
    let platform_json = json!(fingerprint.platform).to_string();
    let mobile_json = json!(fingerprint.mobile).to_string();
    let vendor_json = json!(fingerprint.vendor).to_string();
    let hardware_concurrency_json = json!(fingerprint.hardware_concurrency).to_string();
    let device_memory_json = json!(fingerprint.device_memory).to_string();
    let max_touch_points_json = json!(fingerprint.max_touch_points).to_string();
    let architecture_json = json!(if fingerprint.mobile { "arm" } else { "x86" }).to_string();
    let screen_width_json = json!(screen.screen.width).to_string();
    let screen_height_json = json!(screen.screen.height).to_string();
    let avail_width_json = json!(screen.avail.width).to_string();
    let avail_height_json = json!(screen.avail.height).to_string();
    let color_depth_json = json!(screen.color_depth).to_string();
    let pixel_depth_json = json!(screen.pixel_depth).to_string();
    let device_scale_factor_json = json!(screen.device_scale_factor).to_string();
    let mut lines = Vec::new();

    if config.stealth {
        lines.push(
            "Object.defineProperty(navigator, 'webdriver', { get: () => undefined, configurable: true });"
                .to_string(),
        );
    }

    if config.stealth || device_profile.is_some() {
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
        lines.push(format!(
            "Object.defineProperty(screen, 'width', {{ get: () => {screen_width_json}, configurable: true }});"
        ));
        lines.push(format!(
            "Object.defineProperty(screen, 'height', {{ get: () => {screen_height_json}, configurable: true }});"
        ));
        lines.push(format!(
            "Object.defineProperty(screen, 'availWidth', {{ get: () => {avail_width_json}, configurable: true }});"
        ));
        lines.push(format!(
            "Object.defineProperty(screen, 'availHeight', {{ get: () => {avail_height_json}, configurable: true }});"
        ));
        lines.push(format!(
            "Object.defineProperty(screen, 'colorDepth', {{ get: () => {color_depth_json}, configurable: true }});"
        ));
        lines.push(format!(
            "Object.defineProperty(screen, 'pixelDepth', {{ get: () => {pixel_depth_json}, configurable: true }});"
        ));
        lines.push(format!(
            "Object.defineProperty(window, 'devicePixelRatio', {{ get: () => {device_scale_factor_json}, configurable: true }});"
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
                "Object.defineProperty(navigator, 'userAgentData', {{ get: () => ({{ brands: [{{ brand: 'Chromium', version: '136' }}, {{ brand: 'Not.A/Brand', version: '24' }}], mobile: {mobile_json}, platform: {platform_json}, getHighEntropyValues: async () => ({{ architecture: {architecture_json}, bitness: '64', model: '', platform: {platform_json}, platformVersion: '10.0.0', uaFullVersion: '136.0.0.0' }}) }}), configurable: true }});"
            ));
        }
    }

    let mut sections = Vec::new();
    if !lines.is_empty() {
        sections.push(format!("(() => {{\n{}\n}})();", lines.join("\n")));
    }
    sections.extend(config.stealth_scripts.iter().cloned());

    Some(sections.join("\n"))
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
    viewport: Size,
    stealth: bool,
    stealth_scripts: Vec<String>,
    keep_alive: KeepAlive,
    keep_alive_scope: KeepAliveScope,
    keep_alive_key: Option<String>,
    device_profile: Option<BrowserResolvedDeviceProfile>,
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
            viewport: browser_execution_viewport(execution_plan.device_profile.as_ref()),
            stealth: config.stealth,
            stealth_scripts: config.stealth_scripts.clone(),
            keep_alive: execution_plan.keep_alive,
            keep_alive_scope: execution_plan.keep_alive_scope,
            keep_alive_key: execution_plan.keep_alive_key.clone(),
            device_profile: execution_plan.device_profile.clone(),
            proxy: request.proxy.as_ref().map(|proxy| proxy.url.clone()),
        }
    }
}

#[cfg(any(feature = "browser", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserKeepAliveLifecycle {
    last_returned_at: Instant,
    completed_uses: u64,
}

#[cfg(any(feature = "browser", test))]
#[cfg_attr(all(test, not(feature = "browser")), allow(dead_code))]
impl BrowserKeepAliveLifecycle {
    fn new(now: Instant) -> Self {
        Self {
            last_returned_at: now,
            completed_uses: 0,
        }
    }

    fn record_return(&mut self, now: Instant) {
        self.last_returned_at = now;
        self.completed_uses = self.completed_uses.saturating_add(1);
    }
}

#[cfg(feature = "browser")]
struct BrowserKeepAlive {
    signature: BrowserSessionSignature,
    #[allow(dead_code)]
    // Keeps the underlying Playwright server process alive for this keep_alive entry.
    playwright: Playwright,
    context: BrowserContext,
    page: Option<Page>,
    lifecycle: BrowserKeepAliveLifecycle,
}

#[cfg(feature = "browser")]
async fn fetch_with_playwright_inner(
    request: &Request,
    config: &BrowserConfig,
) -> Result<BrowserFetchResult, SpiderError> {
    let _session_execution_guard = acquire_browser_session_execution_guard(request).await;
    let execution_plan = BrowserExecutionPlan::from_request(request, config).await?;

    if request.session.is_some() && execution_plan.keep_alive != KeepAlive::Isolated {
        return fetch_with_playwright_keep_alive(request, config, &execution_plan).await;
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
        execution_plan.device_profile.as_ref(),
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
async fn fetch_with_playwright_keep_alive(
    request: &Request,
    config: &BrowserConfig,
    execution_plan: &BrowserExecutionPlan,
) -> Result<BrowserFetchResult, SpiderError> {
    let session_id = request
        .session
        .as_ref()
        .map(|session| session.id.clone())
        .ok_or_else(|| SpiderError::download("browser keep_alive requires request.session"))?;
    let cache_key = browser_keep_alive_cache_key(request, execution_plan)?;
    let signature = BrowserSessionSignature::for_request(request, config, execution_plan);
    let mut session = match take_browser_keep_alive(&cache_key, execution_plan).await? {
        Some(session) => {
            if session.signature != signature {
                insert_browser_keep_alive(&cache_key, session).await;
                return Err(browser_keep_alive_mismatch_error(&session_id));
            }
            session
        }
        None => {
            let (playwright, context, _user_data_dir) =
                launch_browser_context(request, config, execution_plan).await?;
            BrowserKeepAlive {
                signature: signature.clone(),
                playwright,
                context,
                page: None,
                lifecycle: BrowserKeepAliveLifecycle::new(Instant::now()),
            }
        }
    };

    let keep_page = execution_plan.keep_alive == KeepAlive::Page;
    let existing_page = if keep_page { session.page.take() } else { None };
    let outcome = run_browser_request_in_context(
        &session.context,
        request,
        config,
        execution_plan.device_profile.as_ref(),
        keep_page,
        existing_page,
    )
    .await;

    match outcome {
        Ok((result, page)) => {
            session.page = page;
            store_browser_keep_alive(&cache_key, session, execution_plan).await?;
            Ok(result)
        }
        Err(error) => {
            session.page = None;
            match execution_plan.keep_alive_on_error {
                KeepAliveOnError::Keep => {
                    store_browser_keep_alive(&cache_key, session, execution_plan).await?;
                }
                KeepAliveOnError::Reset => {
                    close_browser_keep_alive(session).await?;
                }
            }
            Err(error)
        }
    }
}

#[cfg(feature = "browser")]
fn browser_keep_alive_mismatch_error(session_id: &str) -> SpiderError {
    SpiderError::download(format!(
        "browser keep_alive `{session_id}` requires stable engine/headless/device_profile/stealth/stealth_scripts/proxy/keep_alive/keep_alive_scope/keep_alive_key across requests"
    ))
}

#[cfg(any(feature = "browser", test))]
fn browser_session_device_profile_mismatch_error(session_id: &str) -> SpiderError {
    SpiderError::download(format!(
        "browser session `{session_id}` requires stable engine/device_profile/stealth across requests once a browser profile has been established"
    ))
}

#[cfg(any(feature = "browser", test))]
async fn resolve_effective_device_profile(
    request: &Request,
    config: &BrowserConfig,
) -> Result<Option<BrowserResolvedDeviceProfile>, SpiderError> {
    let requested_profile = resolve_device_profile(config)?;
    let Some(session) = &request.session else {
        return Ok(requested_profile);
    };

    let persisted = read_browser_session_device_profile(&session.id).await?;
    match (persisted, requested_profile) {
        (Some(state), Some(requested_profile)) => {
            if state.engine != config.engine || state.device_profile != requested_profile {
                return Err(browser_session_device_profile_mismatch_error(&session.id));
            }

            Ok(Some(state.device_profile))
        }
        (Some(state), None) => {
            if state.engine != config.engine {
                return Err(browser_session_device_profile_mismatch_error(&session.id));
            }

            Ok(Some(state.device_profile))
        }
        (None, Some(requested_profile)) => {
            write_browser_session_device_profile(&session.id, config.engine, &requested_profile)
                .await?;
            Ok(Some(requested_profile))
        }
        (None, None) => Ok(None),
    }
}

#[cfg(any(feature = "browser", test))]
async fn read_browser_session_device_profile(
    session_id: &str,
) -> Result<Option<BrowserSessionDeviceProfileState>, SpiderError> {
    let path = browser_session_device_profile_path(session_id);
    let contents = match tokio::fs::read_to_string(&path).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(SpiderError::download(format!(
                "failed to read browser session device profile: {error}"
            )));
        }
    };

    serde_json::from_str(&contents).map(Some).map_err(|error| {
        SpiderError::download(format!(
            "failed to decode browser session device profile: {error}"
        ))
    })
}

#[cfg(any(feature = "browser", test))]
async fn write_browser_session_device_profile(
    session_id: &str,
    engine: crate::request::browser::Engine,
    device_profile: &BrowserResolvedDeviceProfile,
) -> Result<(), SpiderError> {
    let path = browser_session_device_profile_path(session_id);
    let parent = path.parent().ok_or_else(|| {
        SpiderError::download("browser session device profile path is missing parent directory")
    })?;
    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        SpiderError::download(format!(
            "failed to create browser session device profile dir: {error}"
        ))
    })?;
    let contents = serde_json::to_vec_pretty(&BrowserSessionDeviceProfileState {
        engine,
        device_profile: device_profile.clone(),
    })
    .map_err(|error| {
        SpiderError::download(format!(
            "failed to encode browser session device profile: {error}"
        ))
    })?;
    tokio::fs::write(&path, contents).await.map_err(|error| {
        SpiderError::download(format!(
            "failed to write browser session device profile: {error}"
        ))
    })
}

#[cfg(feature = "browser")]
async fn close_browser_keep_alive(mut session: BrowserKeepAlive) -> Result<(), SpiderError> {
    if let Some(page) = session.page.take()
        && !page.is_closed()
    {
        let _ = page.close().await.map_err(map_playwright_error);
    }

    let close_outcome = session.context.close().await.map_err(map_playwright_error);
    let shutdown_outcome = session
        .playwright
        .shutdown()
        .await
        .map_err(map_playwright_error);

    match close_outcome {
        Ok(()) => shutdown_outcome,
        Err(error) => {
            let _ = shutdown_outcome;
            Err(error)
        }
    }
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
        execution_plan.device_profile.as_ref(),
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
    device_profile: Option<&BrowserResolvedDeviceProfile>,
    keep_page: bool,
    existing_page: Option<Page>,
) -> Result<(BrowserFetchResult, Option<Page>), SpiderError> {
    apply_request_state_to_context(context, request, device_profile).await?;

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
    device_profile: Option<&BrowserResolvedDeviceProfile>,
) -> Result<(), SpiderError> {
    context
        .set_extra_http_headers(build_browser_context_headers(
            request,
            device_profile.map(|profile| &profile.fingerprint),
        ))
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

        if let Some(selector) = &config.wait_for_selector {
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
    device_profile: Option<&BrowserResolvedDeviceProfile>,
) -> Result<BrowserContextOptions, SpiderError> {
    let mut builder = BrowserContextOptions::builder()
        .headless(config.headless)
        .viewport(Viewport {
            width: browser_execution_viewport(device_profile).width,
            height: browser_execution_viewport(device_profile).height,
        });

    if let Some(profile) = device_profile.map(|profile| &profile.fingerprint) {
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

    let headers =
        build_browser_context_headers(request, device_profile.map(|profile| &profile.fingerprint));
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
    profile: Option<&BrowserResolvedFingerprintProfile>,
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
                "browser wait_for_selector timed out: {selector}"
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
fn browser_keep_alive_cache() -> &'static tokio::sync::Mutex<BTreeMap<String, BrowserKeepAlive>> {
    static KEEP_ALIVE: OnceLock<tokio::sync::Mutex<BTreeMap<String, BrowserKeepAlive>>> =
        OnceLock::new();

    KEEP_ALIVE.get_or_init(|| tokio::sync::Mutex::new(BTreeMap::new()))
}

#[cfg(any(feature = "browser", test))]
fn browser_keep_alive_scope_origin(url: &str) -> Result<String, SpiderError> {
    let parsed = Url::parse(url).map_err(|error| {
        SpiderError::download(format!(
            "invalid browser URL for keep_alive_scope=origin: {error}"
        ))
    })?;
    let origin = parsed.origin().ascii_serialization();

    if origin == "null" {
        Ok(url.to_string())
    } else {
        Ok(origin)
    }
}

#[cfg(any(feature = "browser", test))]
fn browser_keep_alive_cache_key(
    request: &Request,
    execution_plan: &BrowserExecutionPlan,
) -> Result<String, SpiderError> {
    let session_id = request
        .session
        .as_ref()
        .map(|session| session.id.as_str())
        .ok_or_else(|| SpiderError::download("browser keep_alive requires request.session"))?;

    let mut key = match execution_plan.keep_alive_scope {
        KeepAliveScope::Session => Ok(session_id.to_string()),
        KeepAliveScope::Origin => {
            let origin = browser_keep_alive_scope_origin(&request.url)?;
            Ok(format!("{session_id}::{origin}"))
        }
    }?;

    if let Some(keep_alive_key) = execution_plan.keep_alive_key.as_deref() {
        key.push_str("::key=");
        key.push_str(keep_alive_key);
    }

    Ok(key)
}

#[cfg(any(feature = "browser", test))]
fn browser_keep_alive_is_expired(
    lifecycle: &BrowserKeepAliveLifecycle,
    execution_plan: &BrowserExecutionPlan,
    now: Instant,
) -> Result<bool, SpiderError> {
    if let Some(max_idle) = execution_plan.keep_alive_max_idle {
        let max_idle = std::time::Duration::try_from(max_idle).map_err(|error| {
            SpiderError::download(format!("invalid browser keep_alive_max_idle: {error}"))
        })?;

        if now.duration_since(lifecycle.last_returned_at) > max_idle {
            return Ok(true);
        }
    }

    if let Some(max_uses) = execution_plan.keep_alive_max_uses
        && lifecycle.completed_uses >= max_uses
    {
        return Ok(true);
    }

    Ok(false)
}

#[cfg(any(feature = "browser", test))]
fn browser_keep_alive_should_store_after_return(
    lifecycle: &BrowserKeepAliveLifecycle,
    execution_plan: &BrowserExecutionPlan,
) -> bool {
    execution_plan
        .keep_alive_max_uses
        .map(|max_uses| lifecycle.completed_uses < max_uses)
        .unwrap_or(true)
}

#[cfg(feature = "browser")]
async fn take_browser_keep_alive(
    cache_key: &str,
    execution_plan: &BrowserExecutionPlan,
) -> Result<Option<BrowserKeepAlive>, SpiderError> {
    let cache = browser_keep_alive_cache();
    let mut cache = cache.lock().await;
    let Some(session) = cache.remove(cache_key) else {
        return Ok(None);
    };
    drop(cache);

    if browser_keep_alive_is_expired(&session.lifecycle, execution_plan, Instant::now())? {
        close_browser_keep_alive(session).await?;
        return Ok(None);
    }

    Ok(Some(session))
}

#[cfg(feature = "browser")]
async fn insert_browser_keep_alive(cache_key: &str, session: BrowserKeepAlive) {
    let cache = browser_keep_alive_cache();
    let mut cache = cache.lock().await;
    cache.insert(cache_key.to_string(), session);
}

#[cfg(feature = "browser")]
async fn store_browser_keep_alive(
    cache_key: &str,
    mut session: BrowserKeepAlive,
    execution_plan: &BrowserExecutionPlan,
) -> Result<(), SpiderError> {
    session.lifecycle.record_return(Instant::now());

    if !browser_keep_alive_should_store_after_return(&session.lifecycle, execution_plan) {
        close_browser_keep_alive(session).await?;
        return Ok(());
    }

    insert_browser_keep_alive(cache_key, session).await;
    Ok(())
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
fn browser_session_device_profile_path(session_id: &str) -> PathBuf {
    browser_session_user_data_dir(session_id)
        .join(".halo-spider")
        .join("device-profile.json")
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
    use crate::request::browser::{
        Config as BrowserConfig, DeviceProfile, Engine as BrowserEngine, FingerprintProfile,
        KeepAlive, KeepAliveOnError, KeepAliveScope, ScreenProfile, Size,
    };
    use jiff::SignedDuration;
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
    fn browser_request_contract_allows_device_profile() {
        let request = Request::browser("https://example.com").with_browser(
            BrowserConfig::default().with_device_profile(
                DeviceProfile::new()
                    .with_fingerprint(
                        FingerprintProfile::new()
                            .with_locale("ja-JP")
                            .with_timezone("Asia/Tokyo")
                            .with_accept_language("ja-JP,ja;q=0.9")
                            .with_languages(["ja-JP", "ja"]),
                    )
                    .with_screen(ScreenProfile::new().with_viewport(1440, 900)),
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
    fn browser_request_contract_allows_partial_device_profile() {
        let request = Request::browser("https://example.com").with_browser(
            BrowserConfig::default().with_device_profile(
                DeviceProfile::new().with_screen(ScreenProfile::new().with_avail(1280, 680)),
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
    fn browser_request_contract_rejects_conflicting_screen_profile() {
        let request = Request::browser("https://example.com").with_browser(
            BrowserConfig::default().with_device_profile(
                DeviceProfile::new().with_screen(
                    ScreenProfile::new()
                        .with_viewport(1440, 900)
                        .with_screen(1366, 768),
                ),
            ),
        );

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
            SpiderError::download("browser device_profile.screen requires screen >= viewport")
        );
    }

    #[test]
    fn browser_request_contract_rejects_live_reuse_without_session() {
        let request = Request::browser("https://example.com")
            .with_browser(BrowserConfig::default().with_keep_alive(KeepAlive::Context));

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
            SpiderError::download("browser keep_alive=context/page requires request.session")
        );
    }

    #[test]
    fn browser_request_contract_allows_live_reuse_with_session() {
        let request = Request::browser("https://example.com")
            .with_session("shared-browser")
            .with_browser(BrowserConfig::default().with_keep_alive(KeepAlive::Page));

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
    fn browser_request_contract_rejects_non_positive_keep_alive_max_idle() {
        let request = Request::browser("https://example.com")
            .with_session("shared-browser")
            .with_browser(
                BrowserConfig::default()
                    .with_keep_alive(KeepAlive::Context)
                    .with_keep_alive_max_idle(SignedDuration::ZERO),
            );

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
            SpiderError::download("browser keep_alive_max_idle must be greater than 0")
        );
    }

    #[test]
    fn browser_keep_alive_cache_key_uses_session_scope_by_default() {
        let request = Request::browser("https://news.example.com/list").with_session("shared");
        let execution_plan = block_on(BrowserExecutionPlan::from_request(
            &request,
            &BrowserConfig::default().with_keep_alive(KeepAlive::Context),
        ))
        .expect("execution plan should build");

        let key = browser_keep_alive_cache_key(&request, &execution_plan)
            .expect("cache key should build");

        assert_eq!(key, "shared");
    }

    #[test]
    fn browser_keep_alive_cache_key_can_scope_by_origin() {
        let request =
            Request::browser("https://news.example.com/list?page=1").with_session("shared");
        let execution_plan = block_on(BrowserExecutionPlan::from_request(
            &request,
            &BrowserConfig::default()
                .with_keep_alive(KeepAlive::Context)
                .with_keep_alive_scope(KeepAliveScope::Origin),
        ))
        .expect("execution plan should build");

        let key = browser_keep_alive_cache_key(&request, &execution_plan)
            .expect("cache key should build");

        assert_eq!(key, "shared::https://news.example.com");
    }

    #[test]
    fn browser_keep_alive_cache_key_can_append_explicit_business_key() {
        let request =
            Request::browser("https://news.example.com/list?page=1").with_session("shared");
        let execution_plan = block_on(BrowserExecutionPlan::from_request(
            &request,
            &BrowserConfig::default()
                .with_keep_alive(KeepAlive::Context)
                .with_keep_alive_scope(KeepAliveScope::Origin)
                .with_keep_alive_key("account:primary"),
        ))
        .expect("execution plan should build");

        let key = browser_keep_alive_cache_key(&request, &execution_plan)
            .expect("cache key should build");

        assert_eq!(key, "shared::https://news.example.com::key=account:primary");
    }

    #[test]
    fn browser_execution_plan_carries_keep_alive_lifecycle_controls() {
        let request = Request::browser("https://news.example.com/list").with_session("shared");
        let execution_plan = block_on(BrowserExecutionPlan::from_request(
            &request,
            &BrowserConfig::default()
                .with_keep_alive(KeepAlive::Page)
                .with_keep_alive_scope(KeepAliveScope::Origin)
                .with_keep_alive_key("account:primary")
                .with_keep_alive_max_idle(SignedDuration::from_secs(30))
                .with_keep_alive_max_uses(12)
                .with_keep_alive_on_error(KeepAliveOnError::Reset),
        ))
        .expect("execution plan should build");

        assert_eq!(execution_plan.keep_alive, KeepAlive::Page);
        assert_eq!(execution_plan.keep_alive_scope, KeepAliveScope::Origin);
        assert_eq!(
            execution_plan.keep_alive_key.as_deref(),
            Some("account:primary")
        );
        assert_eq!(
            execution_plan.keep_alive_max_idle,
            Some(SignedDuration::from_secs(30))
        );
        assert_eq!(execution_plan.keep_alive_max_uses, Some(12));
        assert_eq!(execution_plan.keep_alive_on_error, KeepAliveOnError::Reset);
    }

    #[test]
    fn browser_keep_alive_expires_when_idle_window_passes() {
        let request = Request::browser("https://news.example.com/list").with_session("shared");
        let now = Instant::now();
        let lifecycle = BrowserKeepAliveLifecycle {
            last_returned_at: now
                .checked_sub(std::time::Duration::from_secs(2))
                .expect("instant should support checked subtraction"),
            completed_uses: 1,
        };
        let execution_plan = block_on(BrowserExecutionPlan::from_request(
            &request,
            &BrowserConfig::default()
                .with_keep_alive(KeepAlive::Context)
                .with_keep_alive_max_idle(SignedDuration::from_secs(1)),
        ))
        .expect("execution plan should build");

        let expired = browser_keep_alive_is_expired(&lifecycle, &execution_plan, now)
            .expect("expiry check should succeed");

        assert!(expired);
    }

    #[test]
    fn browser_keep_alive_expires_when_max_uses_is_reached() {
        let request = Request::browser("https://news.example.com/list").with_session("shared");
        let lifecycle = BrowserKeepAliveLifecycle {
            last_returned_at: Instant::now(),
            completed_uses: 3,
        };
        let execution_plan = block_on(BrowserExecutionPlan::from_request(
            &request,
            &BrowserConfig::default()
                .with_keep_alive(KeepAlive::Context)
                .with_keep_alive_max_uses(3),
        ))
        .expect("execution plan should build");

        let expired =
            browser_keep_alive_is_expired(&lifecycle, &execution_plan, lifecycle.last_returned_at)
                .expect("expiry check should succeed");

        assert!(expired);
    }

    #[test]
    fn browser_keep_alive_store_check_stops_reuse_after_max_uses() {
        let request = Request::browser("https://news.example.com/list").with_session("shared");
        let lifecycle = BrowserKeepAliveLifecycle {
            last_returned_at: Instant::now(),
            completed_uses: 4,
        };
        let execution_plan = block_on(BrowserExecutionPlan::from_request(
            &request,
            &BrowserConfig::default()
                .with_keep_alive(KeepAlive::Context)
                .with_keep_alive_max_uses(4),
        ))
        .expect("execution plan should build");

        assert!(!browser_keep_alive_should_store_after_return(
            &lifecycle,
            &execution_plan
        ));
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
            .with_browser(BrowserConfig::default().with_wait_for_selector(".result"));

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
    fn browser_request_contract_allows_external_stealth_script() {
        let request = Request::browser("https://example.com").with_browser(
            BrowserConfig::default().with_stealth_script("window.__thirdPartyStealth = true;"),
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
    fn browser_request_contract_rejects_empty_external_stealth_script() {
        let request = Request::browser("https://example.com")
            .with_browser(BrowserConfig::default().with_stealth_script("   "));

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
            SpiderError::download("browser stealth_scripts must not contain empty scripts")
        );
    }

    #[test]
    fn browser_request_contract_rejects_empty_wait_for_selector() {
        let request = Request::browser("https://example.com")
            .with_browser(BrowserConfig::default().with_wait_for_selector("   "));

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
            SpiderError::download("browser wait_for_selector requires a non-empty selector")
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
    fn resolve_device_profile_compiles_partial_fingerprint_against_engine_defaults() {
        let config = BrowserConfig::default().with_device_profile(
            DeviceProfile::new().with_fingerprint(
                FingerprintProfile::new()
                    .with_locale("zh-CN")
                    .with_timezone("Asia/Shanghai")
                    .with_accept_language("zh-CN,zh;q=0.9,en;q=0.8")
                    .with_languages(["zh-CN", "zh", "en"]),
            ),
        );

        let profile = resolve_device_profile(&config)
            .unwrap()
            .expect("profile should resolve");

        assert_eq!(
            profile.fingerprint.user_agent,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36"
        );
        assert_eq!(profile.fingerprint.locale, "zh-CN");
        assert_eq!(profile.fingerprint.timezone, "Asia/Shanghai");
        assert_eq!(profile.fingerprint.languages, vec!["zh-CN", "zh", "en"]);
    }

    #[test]
    fn resolve_device_profile_uses_engine_specific_fingerprint_defaults() {
        let config = BrowserConfig::default()
            .with_engine(BrowserEngine::Firefox)
            .with_device_profile(DeviceProfile::new());

        let profile = resolve_device_profile(&config)
            .unwrap()
            .expect("profile should resolve");

        assert_eq!(
            profile.fingerprint.user_agent,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:137.0) Gecko/20100101 Firefox/137.0"
        );
        assert_eq!(profile.fingerprint.platform, "Win32");
        assert!(!profile.fingerprint.mobile);
        assert_eq!(profile.fingerprint.vendor, "");
        assert_eq!(profile.fingerprint.locale, "en-US");
        assert_eq!(profile.fingerprint.timezone, "America/New_York");
    }

    #[test]
    fn resolve_device_profile_uses_builtin_profile_for_stealth_requests() {
        let config = BrowserConfig::default().with_stealth(true);

        let profile = resolve_device_profile(&config)
            .unwrap()
            .expect("stealth should resolve a builtin profile");

        assert_eq!(
            profile.fingerprint.user_agent,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36"
        );
        assert_eq!(profile.fingerprint.vendor, "Google Inc.");
        assert_eq!(profile.screen.viewport, Size::new(1280, 720));
    }

    #[test]
    fn resolve_device_profile_uses_mobile_defaults_when_requested() {
        let config = BrowserConfig::default().with_device_profile(
            DeviceProfile::new().with_fingerprint(FingerprintProfile::new().with_mobile(true)),
        );

        let profile = resolve_device_profile(&config)
            .unwrap()
            .expect("mobile profile should resolve");

        assert!(profile.fingerprint.mobile);
        assert_eq!(profile.fingerprint.platform, "Linux armv81");
        assert!(profile.fingerprint.user_agent.contains("Android 14"));
        assert!(profile.fingerprint.user_agent.contains("Mobile"));
        assert_eq!(profile.fingerprint.max_touch_points, 5);
        assert_eq!(profile.screen.viewport, Size::new(390, 844));
        assert_eq!(profile.screen.device_scale_factor, 3);
    }

    #[test]
    fn resolve_device_profile_derives_screen_defaults_and_missing_sizes() {
        let config = BrowserConfig::default().with_device_profile(
            DeviceProfile::new().with_screen(
                ScreenProfile::new()
                    .with_viewport(1440, 900)
                    .with_avail(1728, 1067)
                    .with_color_depth(30),
            ),
        );

        let profile = resolve_device_profile(&config)
            .unwrap()
            .expect("profile should resolve");

        assert_eq!(profile.screen.viewport, Size::new(1440, 900));
        assert_eq!(profile.screen.screen, Size::new(1728, 1067));
        assert_eq!(profile.screen.avail, Size::new(1728, 1067));
        assert_eq!(profile.screen.color_depth, 30);
        assert_eq!(profile.screen.pixel_depth, 24);
        assert_eq!(profile.screen.device_scale_factor, 1);
    }

    #[test]
    fn resolve_device_profile_rejects_conflicting_screen_values() {
        let config = BrowserConfig::default().with_device_profile(
            DeviceProfile::new().with_screen(
                ScreenProfile::new()
                    .with_screen(1366, 768)
                    .with_avail(1600, 900),
            ),
        );

        let error = resolve_device_profile(&config).unwrap_err();

        assert_eq!(
            error,
            SpiderError::download("browser device_profile.screen requires screen >= avail")
        );
    }

    #[test]
    fn browser_execution_plan_pins_first_session_device_profile() {
        let session_id = unique_browser_test_session_id("session-device-profile");
        cleanup_browser_session_artifacts(&session_id);

        let first_request = Request::browser("https://example.com/app").with_session(&session_id);
        let first_plan = block_on(BrowserExecutionPlan::from_request(
            &first_request,
            &BrowserConfig::default().with_stealth(true),
        ))
        .expect("first execution plan should build");

        assert!(first_plan.device_profile.is_some());
        assert!(browser_session_device_profile_path(&session_id).exists());

        let followup_request =
            Request::browser("https://example.com/dashboard").with_session(&session_id);
        let followup_plan = block_on(BrowserExecutionPlan::from_request(
            &followup_request,
            &BrowserConfig::default(),
        ))
        .expect("follow-up execution plan should reuse stored profile");

        assert_eq!(followup_plan.device_profile, first_plan.device_profile);
        assert!(followup_plan.init_script.is_some());

        cleanup_browser_session_artifacts(&session_id);
    }

    #[test]
    fn browser_execution_plan_rejects_conflicting_session_device_profile() {
        let session_id = unique_browser_test_session_id("session-device-conflict");
        cleanup_browser_session_artifacts(&session_id);

        let first_request = Request::browser("https://example.com/app").with_session(&session_id);
        block_on(BrowserExecutionPlan::from_request(
            &first_request,
            &BrowserConfig::default().with_device_profile(
                DeviceProfile::new().with_fingerprint(
                    FingerprintProfile::new()
                        .with_locale("ja-JP")
                        .with_timezone("Asia/Tokyo")
                        .with_accept_language("ja-JP,ja;q=0.9")
                        .with_languages(["ja-JP", "ja"]),
                ),
            ),
        ))
        .expect("first execution plan should build");

        let conflicting_request =
            Request::browser("https://example.com/other").with_session(&session_id);
        let error = block_on(BrowserExecutionPlan::from_request(
            &conflicting_request,
            &BrowserConfig::default().with_device_profile(
                DeviceProfile::new().with_fingerprint(
                    FingerprintProfile::new()
                        .with_locale("en-US")
                        .with_timezone("America/New_York")
                        .with_accept_language("en-US,en;q=0.9")
                        .with_languages(["en-US", "en"]),
                ),
            ),
        ))
        .unwrap_err();

        assert_eq!(
            error,
            SpiderError::download(format!(
                "browser session `{session_id}` requires stable engine/device_profile/stealth across requests once a browser profile has been established"
            ))
        );

        cleanup_browser_session_artifacts(&session_id);
    }

    #[test]
    fn build_browser_init_script_supports_device_profile_and_stealth() {
        let config = BrowserConfig::default()
            .with_stealth(true)
            .with_stealth_script("window.__thirdPartyStealth = true;")
            .with_device_profile(
                DeviceProfile::new()
                    .with_fingerprint(
                        FingerprintProfile::new()
                            .with_locale("en-US")
                            .with_timezone("America/New_York")
                            .with_accept_language("en-US,en;q=0.9")
                            .with_languages(["en-US", "en"]),
                    )
                    .with_screen(
                        ScreenProfile::new()
                            .with_screen(1728, 1117)
                            .with_avail(1728, 1067),
                    ),
            );
        let profile = resolve_device_profile(&config)
            .unwrap()
            .expect("profile should resolve");
        let init_script =
            build_browser_init_script(&config, Some(&profile)).expect("init script should exist");

        assert!(init_script.contains("Object.defineProperty(navigator, 'webdriver'"));
        assert!(init_script.contains("Object.defineProperty(navigator, 'languages'"));
        assert!(init_script.contains("Object.defineProperty(screen, 'width'"));
        assert!(init_script.contains("Object.defineProperty(screen, 'availWidth'"));
        assert!(init_script.contains("Object.defineProperty(window, 'devicePixelRatio'"));
        assert!(init_script.contains("Object.defineProperty(navigator, 'plugins'"));
        assert!(init_script.contains("navigator.permissions.query"));
        assert!(init_script.contains("window.__thirdPartyStealth = true;"));
        assert!(
            init_script
                .find("Object.defineProperty(navigator, 'webdriver'")
                .expect("builtin stealth bootstrap should exist")
                < init_script
                    .find("window.__thirdPartyStealth = true;")
                    .expect("external stealth script should exist")
        );
    }

    #[test]
    fn build_browser_init_script_carries_mobile_hint_for_chromium() {
        let config = BrowserConfig::default()
            .with_stealth(true)
            .with_device_profile(
                DeviceProfile::new().with_fingerprint(FingerprintProfile::new().with_mobile(true)),
            );

        let profile = resolve_device_profile(&config)
            .unwrap()
            .expect("mobile profile should resolve");
        let init_script =
            build_browser_init_script(&config, Some(&profile)).expect("init script should exist");

        assert!(init_script.contains("mobile: true"));
        assert!(init_script.contains("architecture: \"arm\""));
        assert!(init_script.contains("Object.defineProperty(navigator, 'maxTouchPoints'"));
    }

    #[test]
    fn build_browser_init_script_supports_external_stealth_script_without_builtin_stealth() {
        let config =
            BrowserConfig::default().with_stealth_script("window.__externalStealth = true;");

        let init_script =
            build_browser_init_script(&config, None).expect("init script should exist");

        assert_eq!(init_script, "window.__externalStealth = true;");
    }

    #[test]
    fn build_browser_context_headers_prefers_request_header_over_profile_default() {
        let request = Request::browser("https://example.com")
            .with_header("Accept-Language", "fr-FR,fr;q=0.9")
            .with_header("x-token", "abc");
        let profile = resolve_device_profile(
            &BrowserConfig::default().with_device_profile(
                DeviceProfile::new().with_fingerprint(
                    FingerprintProfile::new()
                        .with_locale("zh-CN")
                        .with_timezone("Asia/Shanghai")
                        .with_accept_language("zh-CN,zh;q=0.9,en;q=0.8")
                        .with_languages(["zh-CN", "zh", "en"]),
                ),
            ),
        )
        .unwrap()
        .expect("profile should resolve");

        let headers = build_browser_context_headers(&request, Some(&profile.fingerprint));

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
        let request = Request::browser("https://example.com").with_browser(
            BrowserConfig::default().with_device_profile(
                DeviceProfile::new().with_screen(
                    ScreenProfile::new()
                        .with_viewport(1440, 900)
                        .with_screen(1366, 768),
                ),
            ),
        );

        let error = block_on(downloader.fetch(&request)).unwrap_err();

        assert_eq!(
            error,
            SpiderError::download("browser device_profile.screen requires screen >= viewport")
        );
    }

    #[cfg(feature = "browser")]
    #[test]
    fn build_context_options_matches_browser_contract() {
        let config = BrowserConfig::default()
            .with_headless(false)
            .with_device_profile(
                DeviceProfile::new()
                    .with_fingerprint(
                        FingerprintProfile::new()
                            .with_locale("zh-CN")
                            .with_timezone("Asia/Shanghai")
                            .with_accept_language("zh-CN,zh;q=0.9,en;q=0.8")
                            .with_languages(["zh-CN", "zh", "en"]),
                    )
                    .with_screen(ScreenProfile::new().with_viewport(1440, 900)),
            );
        let request = Request::browser("https://example.com")
            .with_header("Accept-Language", "fr-FR,fr;q=0.9")
            .with_header("x-token", "abc")
            .with_proxy("http://127.0.0.1:8080");
        let profile = resolve_device_profile(&config)
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
        assert_eq!(
            options.user_agent.as_deref(),
            Some(profile.fingerprint.user_agent.as_str())
        );
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

    fn unique_browser_test_session_id(prefix: &str) -> String {
        static COUNTER: std::sync::OnceLock<std::sync::atomic::AtomicU64> =
            std::sync::OnceLock::new();
        let counter = COUNTER.get_or_init(|| std::sync::atomic::AtomicU64::new(1));
        let unique = counter.fetch_add(1, Ordering::Relaxed);

        format!("{prefix}-{unique}")
    }

    fn cleanup_browser_session_artifacts(session_id: &str) {
        let _ = block_on(tokio::fs::remove_dir_all(browser_session_user_data_dir(
            session_id,
        )));
    }
}
