pub mod browser;
pub mod http;

use crate::value::Value;
use browser::Config as BrowserConfig;
use http::Config as HttpConfig;
use std::collections::BTreeMap;

pub type Metadata = BTreeMap<String, Value>;
pub type Headers = BTreeMap<String, Vec<String>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RequestMode {
    #[default]
    Http,
    Browser,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeOverride {
    pub values: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallbackTarget {
    pub name: String,
}

impl CallbackTarget {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub url: String,
    pub mode: RequestMode,
    pub method: String,
    pub headers: Headers,
    pub body: Option<Vec<u8>>,
    pub meta: Metadata,
    pub callback: Option<CallbackTarget>,
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
            meta: Metadata::new(),
            callback: None,
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

    pub fn with_header(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
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

    pub fn with_meta(mut self, key: impl Into<String>, value: Value) -> Self {
        self.meta.insert(key.into(), value);
        self
    }

    pub fn with_callback(mut self, callback: impl Into<String>) -> Self {
        self.callback = Some(CallbackTarget::new(callback));
        self
    }

    pub fn with_dont_filter(mut self, dont_filter: bool) -> Self {
        self.dont_filter = dont_filter;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::browser::{Driver, Engine};

    #[test]
    fn creates_default_http_request() {
        let request = Request::new("https://example.com");

        assert_eq!(request.url, "https://example.com");
        assert_eq!(request.mode, RequestMode::Http);
        assert_eq!(request.method, "GET");
        assert!(request.body.is_none());
        assert!(request.callback.is_none());
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
                .with_engine(Engine::GoogleChrome)
                .with_stealth(true)
                .with_fingerprint_profile("desktop"),
        );

        assert_eq!(request.mode, RequestMode::Browser);
        assert!(request.http.is_none());
        assert_eq!(
            request.browser.as_ref().map(|config| config.engine),
            Some(Engine::GoogleChrome)
        );
        assert_eq!(
            request.browser.as_ref().map(|config| config.driver),
            Some(Driver::Playwright)
        );
        assert_eq!(
            request
                .browser
                .as_ref()
                .and_then(|config| config.fingerprint_profile.as_deref()),
            Some("desktop")
        );
    }
}
