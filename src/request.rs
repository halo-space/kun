pub mod browser;
pub mod http;

use crate::error::SpiderError;
use crate::middleware::{
    AUTO_THROTTLE, CONCURRENCY, DEDUP, INTERVAL, RATE_LIMIT, RETRY_BY_ERROR, RETRY_BY_STATUS,
};
use crate::value::Value;
use browser::Config as BrowserConfig;
use http::Config as HttpConfig;
use jiff::SignedDuration;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

pub type Metadata = BTreeMap<String, Value>;
pub type Headers = BTreeMap<String, Vec<String>>;
pub type Cookies = BTreeMap<String, String>;
pub type MiddlewareOptions = BTreeMap<String, Value>;
pub type MiddlewareOverrides = BTreeMap<String, MiddlewareOverride>;

const DEFAULT_ENCODING: &str = "utf-8";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestMode {
    #[default]
    Http,
    Browser,
}

impl RequestMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Browser => "browser",
        }
    }
}

impl Display for RequestMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for RequestMode {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "http" => Ok(Self::Http),
            "browser" => Ok(Self::Browser),
            other => Err(format!("unsupported request mode: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MiddlewareUse {
    pub options: MiddlewareOptions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i32>,
}

impl MiddlewareUse {
    pub fn new(options: MiddlewareOptions) -> Self {
        Self {
            options,
            order: None,
        }
    }

    pub fn with_order(options: MiddlewareOptions, order: i32) -> Self {
        Self {
            options,
            order: Some(order),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MiddlewareOverride {
    Use(MiddlewareUse),
    Skip,
}

pub trait IntoMiddlewareOptions {
    fn into_options(self) -> MiddlewareOptions;
}

impl IntoMiddlewareOptions for MiddlewareOptions {
    fn into_options(self) -> MiddlewareOptions {
        self
    }
}

pub trait FromMiddlewareOptions: Sized {
    fn from_options(options: &MiddlewareOptions) -> Result<Self, SpiderError>;
}

pub trait MiddlewareKey {
    const NAME: &'static str;
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallbackTarget {
    pub name: String,
}

impl CallbackTarget {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub url: String,
}

impl ProxyConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionConfig {
    pub id: String,
}

impl SessionConfig {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub url: String,
    pub mode: RequestMode,
    pub encoding: String,
    pub method: String,
    pub headers: Headers,
    pub body: Option<Vec<u8>>,
    pub cookies: Cookies,
    #[serde(default, with = "option_signed_duration_millis")]
    pub timeout: Option<SignedDuration>,
    pub proxy: Option<ProxyConfig>,
    pub session: Option<SessionConfig>,
    pub priority: i32,
    #[serde(default)]
    pub flags: Vec<String>,
    pub meta: Metadata,
    #[serde(default)]
    pub cb_kwargs: Metadata,
    pub callback: Option<CallbackTarget>,
    #[serde(default)]
    pub errback: Option<CallbackTarget>,
    #[serde(default)]
    pub middleware: MiddlewareOverrides,
    #[serde(default)]
    pub skip_domain_filter: bool,
    pub http: Option<HttpConfig>,
    pub browser: Option<BrowserConfig>,
}

impl Request {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            mode: RequestMode::default(),
            encoding: DEFAULT_ENCODING.to_string(),
            method: "GET".to_string(),
            headers: Headers::new(),
            body: None,
            cookies: Cookies::new(),
            timeout: None,
            proxy: None,
            session: None,
            priority: 0,
            flags: Vec::new(),
            meta: Metadata::new(),
            cb_kwargs: Metadata::new(),
            callback: None,
            errback: None,
            middleware: MiddlewareOverrides::new(),
            skip_domain_filter: false,
            http: Some(HttpConfig::default()),
            browser: None,
        }
    }

    pub fn browser(url: impl Into<String>) -> Self {
        let mut request = Self::new(url);
        request.mode = RequestMode::Browser;
        request.sync_mode_config();
        request
    }

    pub fn with_mode(mut self, mode: RequestMode) -> Self {
        self.mode = mode;
        self.sync_mode_config();
        self
    }

    pub fn with_encoding(mut self, encoding: impl Into<String>) -> Self {
        self.encoding = encoding.into();
        self
    }

    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.method = method.into();
        self
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers
            .entry(name.into())
            .or_default()
            .push(value.into());
        self
    }

    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn with_timeout(mut self, timeout: SignedDuration) -> Self {
        self.timeout = Some(non_negative_duration(timeout));
        self
    }

    pub fn with_proxy(mut self, url: impl Into<String>) -> Self {
        self.proxy = Some(ProxyConfig::new(url));
        self
    }

    pub fn with_proxy_config(mut self, proxy: ProxyConfig) -> Self {
        self.proxy = Some(proxy);
        self
    }

    pub fn with_session(mut self, id: impl Into<String>) -> Self {
        self.session = Some(SessionConfig::new(id));
        self
    }

    pub fn with_session_config(mut self, session: SessionConfig) -> Self {
        self.session = Some(session);
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_flag(mut self, flag: impl Into<String>) -> Self {
        self.flags.push(flag.into());
        self
    }

    pub fn with_flags<I, S>(mut self, flags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.flags.extend(flags.into_iter().map(Into::into));
        self
    }

    pub fn with_meta(mut self, key: impl Into<String>, value: Value) -> Self {
        self.meta.insert(key.into(), value);
        self
    }

    pub fn with_meta_map(mut self, meta: Metadata) -> Self {
        self.meta.extend(meta);
        self
    }

    pub fn with_kwarg(mut self, key: impl Into<String>, value: Value) -> Self {
        self.cb_kwargs.insert(key.into(), value);
        self
    }

    pub fn with_cb_kwargs(mut self, cb_kwargs: Metadata) -> Self {
        self.cb_kwargs.extend(cb_kwargs);
        self
    }

    pub fn to(self, callback: impl Into<String>) -> Self {
        self.with_callback(callback)
    }

    pub fn with_callback(mut self, callback: impl Into<String>) -> Self {
        self.callback = Some(CallbackTarget::new(callback));
        self
    }

    pub fn with_errback(mut self, errback: impl Into<String>) -> Self {
        self.errback = Some(CallbackTarget::new(errback));
        self
    }

    pub fn with_cookie(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.cookies.insert(key.into(), value.into());
        self
    }

    pub fn with_cookies(mut self, cookies: BTreeMap<String, String>) -> Self {
        self.cookies.extend(cookies);
        self
    }

    pub fn with_middleware_options(
        mut self,
        name: impl Into<String>,
        options: impl IntoMiddlewareOptions,
    ) -> Self {
        self.middleware.insert(
            name.into(),
            MiddlewareOverride::Use(MiddlewareUse::new(options.into_options())),
        );
        self
    }

    pub fn with_middleware_options_ordered(
        mut self,
        name: impl Into<String>,
        options: impl IntoMiddlewareOptions,
        order: i32,
    ) -> Self {
        self.middleware.insert(
            name.into(),
            MiddlewareOverride::Use(MiddlewareUse::with_order(options.into_options(), order)),
        );
        self
    }

    pub fn with_middleware<M, C>(self, config: C) -> Self
    where
        M: MiddlewareKey,
        C: IntoMiddlewareOptions,
    {
        self.with_middleware_options(M::NAME, config)
    }

    pub fn with_dedup<C>(self, config: C) -> Self
    where
        C: IntoMiddlewareOptions,
    {
        self.with_middleware_options(DEDUP, config)
    }

    pub fn with_concurrency<C>(self, config: C, order: i32) -> Self
    where
        C: IntoMiddlewareOptions,
    {
        self.with_middleware_options_ordered(CONCURRENCY, config, order)
    }

    pub fn with_interval<C>(self, config: C, order: i32) -> Self
    where
        C: IntoMiddlewareOptions,
    {
        self.with_middleware_options_ordered(INTERVAL, config, order)
    }

    pub fn with_rate_limit<C>(self, config: C, order: i32) -> Self
    where
        C: IntoMiddlewareOptions,
    {
        self.with_middleware_options_ordered(RATE_LIMIT, config, order)
    }

    pub fn with_auto_throttle<C>(self, config: C, order: i32) -> Self
    where
        C: IntoMiddlewareOptions,
    {
        self.with_middleware_options_ordered(AUTO_THROTTLE, config, order)
    }

    pub fn with_retry_by_status<C>(self, config: C, order: i32) -> Self
    where
        C: IntoMiddlewareOptions,
    {
        self.with_middleware_options_ordered(RETRY_BY_STATUS, config, order)
    }

    pub fn with_retry_by_error<C>(self, config: C, order: i32) -> Self
    where
        C: IntoMiddlewareOptions,
    {
        self.with_middleware_options_ordered(RETRY_BY_ERROR, config, order)
    }

    pub fn with_skip_domain_filter(mut self, skip: bool) -> Self {
        self.skip_domain_filter = skip;
        self
    }

    pub fn skip_domain_filter(self) -> Self {
        self.with_skip_domain_filter(true)
    }

    pub fn skips_domain_filter(&self) -> bool {
        self.skip_domain_filter
    }

    pub fn skip<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for name in names {
            self.middleware
                .insert(name.as_ref().to_string(), MiddlewareOverride::Skip);
        }
        self
    }

    pub fn middleware_override(&self, name: &str) -> Option<&MiddlewareOverride> {
        self.middleware.get(name)
    }

    pub fn middleware_override_any<'a>(&'a self, names: &[&str]) -> Option<&'a MiddlewareOverride> {
        names.iter().find_map(|name| self.middleware.get(*name))
    }

    pub fn middleware_options(&self, name: &str) -> Option<&MiddlewareOptions> {
        match self.middleware_override(name) {
            Some(MiddlewareOverride::Use(config)) => Some(&config.options),
            _ => None,
        }
    }

    pub fn middleware_options_any<'a>(&'a self, names: &[&str]) -> Option<&'a MiddlewareOptions> {
        match self.middleware_override_any(names) {
            Some(MiddlewareOverride::Use(config)) => Some(&config.options),
            _ => None,
        }
    }

    pub fn middleware_order(&self, name: &str) -> Option<i32> {
        match self.middleware_override(name) {
            Some(MiddlewareOverride::Use(config)) => config.order,
            _ => None,
        }
    }

    pub fn middleware_skips(&self, name: &str) -> bool {
        matches!(
            self.middleware_override(name),
            Some(MiddlewareOverride::Skip)
        )
    }

    pub fn middleware_skips_any(&self, names: &[&str]) -> bool {
        matches!(
            self.middleware_override_any(names),
            Some(MiddlewareOverride::Skip)
        )
    }

    pub fn with_http(mut self, http: HttpConfig) -> Self {
        self.http = Some(http);
        self.mode = RequestMode::Http;
        self.sync_mode_config();
        self
    }

    pub fn with_browser(mut self, browser: BrowserConfig) -> Self {
        self.browser = Some(browser);
        self.mode = RequestMode::Browser;
        self.sync_mode_config();
        self
    }

    pub fn http_mut(&mut self) -> &mut HttpConfig {
        self.mode = RequestMode::Http;
        if self.http.is_none() {
            self.http = Some(HttpConfig::default());
        }
        self.browser = None;

        self.http.as_mut().expect("http config must exist")
    }

    pub fn browser_mut(&mut self) -> &mut BrowserConfig {
        self.mode = RequestMode::Browser;
        if self.browser.is_none() {
            self.browser = Some(BrowserConfig::default());
        }
        self.http = None;

        self.browser.as_mut().expect("browser config must exist")
    }

    pub fn merge_meta(mut self, patch: &Metadata) -> Self {
        for (key, value) in patch {
            self.meta.insert(key.clone(), value.clone());
        }
        self
    }

    pub fn from_parent_for_follow(parent: &Self, url: impl Into<String>) -> Self {
        let mut request = Self {
            url: url.into(),
            mode: parent.mode,
            encoding: parent.encoding.clone(),
            method: "GET".to_string(),
            headers: parent.headers.clone(),
            body: None,
            cookies: parent.cookies.clone(),
            timeout: parent.timeout,
            proxy: parent.proxy.clone(),
            session: parent.session.clone(),
            priority: 0,
            flags: Vec::new(),
            meta: Metadata::new(),
            cb_kwargs: Metadata::new(),
            callback: None,
            errback: None,
            middleware: MiddlewareOverrides::new(),
            skip_domain_filter: false,
            http: parent.http.clone(),
            browser: parent.browser.clone(),
        };

        if let Some(http) = request.http.as_mut() {
            http.query.clear();
        }

        request.sync_mode_config();
        request
    }

    fn sync_mode_config(&mut self) {
        match self.mode {
            RequestMode::Http => {
                if self.http.is_none() {
                    self.http = Some(HttpConfig::default());
                }
                self.browser = None;
            }
            RequestMode::Browser => {
                if self.browser.is_none() {
                    self.browser = Some(BrowserConfig::default());
                }
                self.http = None;
            }
        }
    }
}

fn non_negative_duration(duration: SignedDuration) -> SignedDuration {
    if duration.is_negative() {
        SignedDuration::ZERO
    } else {
        duration
    }
}

mod option_signed_duration_millis {
    use jiff::SignedDuration;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<SignedDuration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(duration) => {
                let millis =
                    i64::try_from(duration.as_millis()).map_err(serde::ser::Error::custom)?;
                serializer.serialize_some(&millis)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<SignedDuration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<i64>::deserialize(deserializer).map(|value| value.map(SignedDuration::from_millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::{
        AUTO_THROTTLE, CONCURRENCY, INTERVAL, RATE_LIMIT, RETRY_BY_ERROR, RETRY_BY_STATUS,
    };
    use crate::request::browser::{
        ClientHintsProfile, DeviceProfile, Driver, Engine, FingerprintProfile, KeepAlive,
        KeepAliveScope, ScreenProfile, Size,
    };
    use crate::value::Value;
    use jiff::SignedDuration;

    #[test]
    fn creates_default_http_request() {
        let request = Request::new("https://example.com");

        assert_eq!(request.url, "https://example.com");
        assert_eq!(request.mode, RequestMode::Http);
        assert_eq!(request.encoding, "utf-8");
        assert_eq!(request.method, "GET");
        assert!(request.body.is_none());
        assert!(request.cookies.is_empty());
        assert!(request.timeout.is_none());
        assert!(request.proxy.is_none());
        assert!(request.session.is_none());
        assert_eq!(request.priority, 0);
        assert!(request.flags.is_empty());
        assert!(request.cb_kwargs.is_empty());
        assert!(request.callback.is_none());
        assert!(request.errback.is_none());
        assert!(request.middleware.is_empty());
        assert!(!request.skip_domain_filter);
        assert!(request.http.is_some());
        assert!(request.browser.is_none());
    }

    #[test]
    fn creates_browser_request() {
        let request = Request::browser("https://example.com");

        assert_eq!(request.mode, RequestMode::Browser);
        assert!(request.http.is_none());
        assert!(request.browser.is_some());
    }

    #[test]
    fn browser_config_switches_request_mode() {
        let request = Request::new("https://example.com").with_browser(
            BrowserConfig::default()
                .with_driver(Driver::Playwright)
                .with_engine(Engine::Firefox)
                .with_stealth(true)
                .with_device_profile(
                    DeviceProfile::new().with_fingerprint(
                        FingerprintProfile::new()
                            .with_locale("en-US")
                            .with_timezone("America/New_York")
                            .with_accept_language("en-US,en;q=0.9")
                            .with_languages(["en-US", "en"])
                            .with_mobile(true)
                            .with_client_hints(
                                ClientHintsProfile::new()
                                    .with_architecture("arm")
                                    .with_platform_version("14.0.0"),
                            ),
                    ),
                ),
        );

        assert_eq!(request.mode, RequestMode::Browser);
        assert!(request.http.is_none());
        assert_eq!(
            request.browser.as_ref().map(|config| config.engine),
            Some(Engine::Firefox)
        );
        assert_eq!(
            request.browser.as_ref().map(|config| config.driver),
            Some(Driver::Playwright)
        );
        assert_eq!(
            request
                .browser
                .as_ref()
                .and_then(|config| config.device_profile.as_ref())
                .and_then(|profile| profile.fingerprint.as_ref())
                .and_then(|fingerprint| fingerprint.locale.as_deref()),
            Some("en-US")
        );
        assert_eq!(
            request
                .browser
                .as_ref()
                .and_then(|config| config.device_profile.as_ref())
                .and_then(|profile| profile.fingerprint.as_ref())
                .and_then(|fingerprint| fingerprint.mobile),
            Some(true)
        );
        assert_eq!(
            request
                .browser
                .as_ref()
                .and_then(|config| config.device_profile.as_ref())
                .and_then(|profile| profile.fingerprint.as_ref())
                .and_then(|fingerprint| fingerprint.client_hints.as_ref())
                .and_then(|client_hints| client_hints.architecture.as_deref()),
            Some("arm")
        );
    }

    #[test]
    fn request_cookies_are_shared_without_switching_browser_mode() {
        let request = Request::browser("https://example.com").with_cookie("sid", "cookie-1");

        assert_eq!(request.mode, RequestMode::Browser);
        assert_eq!(
            request.cookies.get("sid").map(String::as_str),
            Some("cookie-1")
        );
        assert!(request.http.is_none());
        assert!(request.browser.is_some());
    }

    #[test]
    fn request_supports_core_timeout_proxy_and_session_config() {
        let request = Request::new("https://example.com")
            .with_timeout(SignedDuration::from_secs(5))
            .with_proxy("http://127.0.0.1:8080")
            .with_session("news-session");

        assert_eq!(request.timeout, Some(SignedDuration::from_secs(5)));
        assert_eq!(
            request.proxy,
            Some(ProxyConfig::new("http://127.0.0.1:8080"))
        );
        assert_eq!(request.session, Some(SessionConfig::new("news-session")));
    }

    #[test]
    fn request_from_parent_for_follow_inherits_core_request_semantics() {
        let parent = Request::new("https://example.com/list?page=1")
            .with_method("POST")
            .with_body("payload")
            .with_header("x-token", "abc")
            .with_cookie("sid", "cookie-1")
            .with_timeout(SignedDuration::from_secs(3))
            .with_proxy("http://proxy.internal:8080")
            .with_session("session-a")
            .with_encoding("gbk")
            .with_priority(12)
            .with_flag("list")
            .with_kwarg("page", Value::Number(2.0))
            .with_callback("parse_list")
            .with_errback("handle_error")
            .skip([DEDUP])
            .skip_domain_filter()
            .with_middleware_options(
                "custom_header",
                BTreeMap::from([("name".to_string(), Value::String("x-token".to_string()))]),
            );
        let child = Request::from_parent_for_follow(&parent, "https://example.com/detail");

        assert_eq!(child.url, "https://example.com/detail");
        assert_eq!(child.mode, RequestMode::Http);
        assert_eq!(child.encoding, "gbk");
        assert_eq!(child.method, "GET");
        assert!(child.body.is_none());
        assert_eq!(child.headers.get("x-token"), Some(&vec!["abc".to_string()]));
        assert_eq!(
            child.cookies,
            BTreeMap::from([("sid".to_string(), "cookie-1".to_string())])
        );
        assert_eq!(child.timeout, Some(SignedDuration::from_secs(3)));
        assert_eq!(
            child.proxy,
            Some(ProxyConfig::new("http://proxy.internal:8080"))
        );
        assert_eq!(child.session, Some(SessionConfig::new("session-a")));
        assert_eq!(child.priority, 0);
        assert!(child.flags.is_empty());
        assert!(child.cb_kwargs.is_empty());
        assert!(child.callback.is_none());
        assert!(child.errback.is_none());
        assert!(child.middleware.is_empty());
        assert!(!child.middleware_skips(DEDUP));
        assert!(!child.skips_domain_filter());
        assert!(
            child
                .http
                .as_ref()
                .is_some_and(|http| http.query.is_empty())
        );
    }

    #[test]
    fn request_round_trips_through_serde_for_scheduler_state() {
        let request = Request::browser("https://example.com/render")
            .with_method("POST")
            .with_body("payload")
            .with_header("x-trace", "abc")
            .with_cookie("sid", "cookie-1")
            .with_timeout(SignedDuration::from_secs(5))
            .with_proxy("http://127.0.0.1:8080")
            .with_session("shared-browser")
            .with_encoding("utf-8")
            .with_priority(8)
            .with_flag("seed")
            .with_meta("page", Value::Number(2.0))
            .with_skip_domain_filter(true)
            .with_kwarg("edition", Value::String("morning".to_string()))
            .with_callback("parse_detail")
            .with_errback("handle_error")
            .with_middleware_options(
                "retry",
                BTreeMap::from([("count".to_string(), Value::Number(3.0))]),
            )
            .with_browser(
                BrowserConfig::default()
                    .with_driver(Driver::Playwright)
                    .with_engine(Engine::Chromium)
                    .with_stealth(true)
                    .with_device_profile(
                        DeviceProfile::new()
                            .with_fingerprint(
                                FingerprintProfile::new()
                                    .with_locale("zh-CN")
                                    .with_timezone("Asia/Shanghai")
                                    .with_accept_language("zh-CN,zh;q=0.9,en;q=0.8")
                                    .with_languages(["zh-CN", "zh", "en"])
                                    .with_mobile(true)
                                    .with_client_hints(
                                        ClientHintsProfile::new()
                                            .with_architecture("arm")
                                            .with_platform_version("14.0.0")
                                            .with_ua_full_version("136.0.0.0"),
                                    ),
                            )
                            .with_screen(
                                ScreenProfile::new()
                                    .with_viewport(1440, 900)
                                    .with_screen(1728, 1117)
                                    .with_avail(1728, 1067),
                            ),
                    )
                    .with_keep_alive(KeepAlive::Context)
                    .with_keep_alive_scope(KeepAliveScope::Origin)
                    .with_wait_for_selector("#app"),
            );

        let encoded = serde_json::to_vec(&request).expect("request should serialize");
        let decoded: Request =
            serde_json::from_slice(&encoded).expect("request should deserialize");

        assert_eq!(decoded.url, "https://example.com/render");
        assert_eq!(decoded.mode, RequestMode::Browser);
        assert_eq!(decoded.encoding, "utf-8");
        assert_eq!(decoded.method, "POST");
        assert_eq!(decoded.body, Some(b"payload".to_vec()));
        assert_eq!(decoded.timeout, Some(SignedDuration::from_secs(5)));
        assert_eq!(
            decoded.proxy,
            Some(ProxyConfig::new("http://127.0.0.1:8080"))
        );
        assert_eq!(decoded.session, Some(SessionConfig::new("shared-browser")));
        assert_eq!(decoded.priority, 8);
        assert_eq!(decoded.flags, vec!["seed".to_string()]);
        assert_eq!(decoded.meta.get("page"), Some(&Value::Number(2.0)));
        assert!(decoded.skips_domain_filter());
        assert_eq!(
            decoded.cb_kwargs.get("edition"),
            Some(&Value::String("morning".to_string()))
        );
        assert_eq!(
            decoded
                .callback
                .as_ref()
                .map(|callback| callback.name.as_str()),
            Some("parse_detail")
        );
        assert_eq!(
            decoded
                .errback
                .as_ref()
                .map(|errback| errback.name.as_str()),
            Some("handle_error")
        );
        assert_eq!(
            decoded.middleware.get("retry"),
            Some(&MiddlewareOverride::Use(MiddlewareUse::new(
                BTreeMap::from([("count".to_string(), Value::Number(3.0))])
            )))
        );
        assert_eq!(
            decoded
                .browser
                .as_ref()
                .and_then(|config| config.wait_for_selector.as_deref()),
            Some("#app")
        );
        assert_eq!(
            decoded.browser.as_ref().map(|config| config.keep_alive),
            Some(KeepAlive::Context)
        );
        assert_eq!(
            decoded
                .browser
                .as_ref()
                .map(|config| config.keep_alive_scope),
            Some(KeepAliveScope::Origin)
        );
        assert_eq!(
            decoded
                .browser
                .as_ref()
                .and_then(|config| config.device_profile.as_ref())
                .and_then(|profile| profile.fingerprint.as_ref())
                .and_then(|profile| profile.timezone.as_deref()),
            Some("Asia/Shanghai")
        );
        assert_eq!(
            decoded
                .browser
                .as_ref()
                .and_then(|config| config.device_profile.as_ref())
                .and_then(|profile| profile.fingerprint.as_ref())
                .and_then(|profile| profile.mobile),
            Some(true)
        );
        assert_eq!(
            decoded
                .browser
                .as_ref()
                .and_then(|config| config.device_profile.as_ref())
                .and_then(|profile| profile.fingerprint.as_ref())
                .and_then(|profile| profile.client_hints.as_ref())
                .and_then(|client_hints| client_hints.ua_full_version.as_deref()),
            Some("136.0.0.0")
        );
        assert_eq!(
            decoded
                .browser
                .as_ref()
                .and_then(|config| config.device_profile.as_ref())
                .and_then(|profile| profile.screen.as_ref())
                .and_then(|screen| screen.viewport.as_ref())
                .cloned(),
            Some(Size::new(1440, 900))
        );
    }

    #[test]
    fn request_expands_retry_and_limits_overrides_to_concrete_middlewares() {
        let request = Request::new("https://example.com")
            .with_retry_by_status(
                BTreeMap::from([("count".to_string(), Value::Number(2.0))]),
                200,
            )
            .with_retry_by_error(
                BTreeMap::from([("count".to_string(), Value::Number(2.0))]),
                210,
            )
            .with_concurrency(
                BTreeMap::from([("concurrency".to_string(), Value::Number(2.0))]),
                225,
            )
            .with_interval(
                BTreeMap::from([("interval".to_string(), Value::Number(300.0))]),
                120,
            )
            .with_rate_limit(
                BTreeMap::from([("rate_per_minute".to_string(), Value::Number(60.0))]),
                130,
            )
            .skip([INTERVAL]);

        assert_eq!(
            request.middleware_options(RETRY_BY_STATUS),
            Some(&BTreeMap::from([
                ("count".to_string(), Value::Number(2.0),)
            ]))
        );
        assert_eq!(
            request.middleware_options(RETRY_BY_ERROR),
            Some(&BTreeMap::from([
                ("count".to_string(), Value::Number(2.0),)
            ]))
        );
        assert!(request.middleware_options(CONCURRENCY).is_some());
        assert!(request.middleware_options(INTERVAL).is_none());
        assert!(request.middleware_options(RATE_LIMIT).is_some());
        assert!(request.middleware_skips(INTERVAL));
        assert_eq!(request.middleware_order(RETRY_BY_STATUS), Some(200));
        assert_eq!(request.middleware_order(RETRY_BY_ERROR), Some(210));
        assert_eq!(request.middleware_order(CONCURRENCY), Some(225));
    }

    #[test]
    fn request_can_mark_dedup_and_domain_bypass_explicitly() {
        let request = Request::new("https://example.com")
            .skip([DEDUP])
            .skip_domain_filter();

        assert!(request.middleware_skips(DEDUP));
        assert!(request.skips_domain_filter());
    }

    #[test]
    fn request_can_clear_domain_bypass_flag() {
        let request = Request::new("https://example.com")
            .skip_domain_filter()
            .with_skip_domain_filter(false);

        assert!(!request.skips_domain_filter());
    }

    #[test]
    fn request_can_set_auto_throttle_with_explicit_order() {
        let request = Request::new("https://example.com").with_auto_throttle(
            BTreeMap::from([
                ("interval".to_string(), Value::Number(300.0)),
                ("target_concurrency".to_string(), Value::Number(2.0)),
            ]),
            120,
        );

        assert!(request.middleware_options(AUTO_THROTTLE).is_some());
        assert_eq!(request.middleware_order(AUTO_THROTTLE), Some(120));
    }
}
