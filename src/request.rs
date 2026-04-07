pub mod browser;
pub mod http;

use crate::value::Value;
use browser::Config as BrowserConfig;
use http::Config as HttpConfig;
use jiff::SignedDuration;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

pub type Metadata = BTreeMap<String, Value>;
pub type Headers = BTreeMap<String, Vec<String>>;

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeOverride {
    pub values: BTreeMap<String, Value>,
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
    pub method: String,
    pub headers: Headers,
    pub body: Option<Vec<u8>>,
    pub cookies: BTreeMap<String, String>,
    #[serde(default, with = "option_signed_duration_millis")]
    pub timeout: Option<SignedDuration>,
    pub proxy: Option<ProxyConfig>,
    pub session: Option<SessionConfig>,
    pub meta: Metadata,
    #[serde(default)]
    pub kwargs: Metadata,
    pub callback: Option<CallbackTarget>,
    #[serde(default)]
    pub errback: Option<CallbackTarget>,
    pub dont_filter: bool,
    pub runtime: Option<RuntimeOverride>,
    pub http: Option<HttpConfig>,
    pub browser: Option<BrowserConfig>,
}

impl Request {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            mode: RequestMode::default(),
            method: "GET".to_string(),
            headers: Headers::new(),
            body: None,
            cookies: BTreeMap::new(),
            timeout: None,
            proxy: None,
            session: None,
            meta: Metadata::new(),
            kwargs: Metadata::new(),
            callback: None,
            errback: None,
            dont_filter: false,
            runtime: None,
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

    pub fn with_meta(mut self, key: impl Into<String>, value: Value) -> Self {
        self.meta.insert(key.into(), value);
        self
    }

    pub fn with_kwarg(mut self, key: impl Into<String>, value: Value) -> Self {
        self.kwargs.insert(key.into(), value);
        self
    }

    pub fn with_kwargs(mut self, kwargs: Metadata) -> Self {
        self.kwargs.extend(kwargs);
        self
    }

    pub fn with_callback(mut self, callback: impl Into<String>) -> Self {
        self.callback = Some(CallbackTarget::new(callback));
        self
    }

    pub fn with_errback(mut self, errback: impl Into<String>) -> Self {
        self.errback = Some(CallbackTarget::new(errback));
        self
    }

    pub fn with_dont_filter(mut self, dont_filter: bool) -> Self {
        self.dont_filter = dont_filter;
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
            method: "GET".to_string(),
            headers: parent.headers.clone(),
            body: None,
            cookies: parent.cookies.clone(),
            timeout: parent.timeout,
            proxy: parent.proxy.clone(),
            session: parent.session.clone(),
            meta: Metadata::new(),
            kwargs: Metadata::new(),
            callback: None,
            errback: None,
            dont_filter: false,
            runtime: parent.runtime.clone(),
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
    use crate::request::browser::{Driver, Engine, FingerprintProfile, KeepAlive, KeepAliveScope};
    use crate::value::Value;
    use jiff::SignedDuration;

    #[test]
    fn creates_default_http_request() {
        let request = Request::new("https://example.com");

        assert_eq!(request.url, "https://example.com");
        assert_eq!(request.mode, RequestMode::Http);
        assert_eq!(request.method, "GET");
        assert!(request.body.is_none());
        assert!(request.cookies.is_empty());
        assert!(request.timeout.is_none());
        assert!(request.proxy.is_none());
        assert!(request.session.is_none());
        assert!(request.kwargs.is_empty());
        assert!(request.callback.is_none());
        assert!(request.errback.is_none());
        assert!(!request.dont_filter);
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
                .with_fingerprint_preset("desktop_en_us"),
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
                .and_then(|config| config.fingerprint_preset.as_deref()),
            Some("desktop_en_us")
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
            .with_kwarg("page", Value::Number(2.0))
            .with_dont_filter(true)
            .with_callback("parse_list")
            .with_errback("handle_error");
        let child = Request::from_parent_for_follow(&parent, "https://example.com/detail");

        assert_eq!(child.url, "https://example.com/detail");
        assert_eq!(child.mode, RequestMode::Http);
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
        assert!(!child.dont_filter);
        assert!(child.kwargs.is_empty());
        assert!(child.callback.is_none());
        assert!(child.errback.is_none());
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
            .with_meta("page", Value::Number(2.0))
            .with_kwarg("edition", Value::String("morning".to_string()))
            .with_callback("parse_detail")
            .with_errback("handle_error")
            .with_browser(
                BrowserConfig::default()
                    .with_driver(Driver::Playwright)
                    .with_engine(Engine::Chromium)
                    .with_stealth(true)
                    .with_fingerprint_profile(
                        FingerprintProfile::new()
                            .with_locale("zh-CN")
                            .with_timezone("Asia/Shanghai")
                            .with_accept_language("zh-CN,zh;q=0.9,en;q=0.8")
                            .with_languages(["zh-CN", "zh", "en"]),
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
        assert_eq!(decoded.method, "POST");
        assert_eq!(decoded.body, Some(b"payload".to_vec()));
        assert_eq!(decoded.timeout, Some(SignedDuration::from_secs(5)));
        assert_eq!(
            decoded.proxy,
            Some(ProxyConfig::new("http://127.0.0.1:8080"))
        );
        assert_eq!(decoded.session, Some(SessionConfig::new("shared-browser")));
        assert_eq!(decoded.meta.get("page"), Some(&Value::Number(2.0)));
        assert_eq!(
            decoded.kwargs.get("edition"),
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
                .and_then(|config| config.fingerprint_profile.as_ref())
                .map(|profile| profile.timezone.as_str()),
            Some("Asia/Shanghai")
        );
    }
}
