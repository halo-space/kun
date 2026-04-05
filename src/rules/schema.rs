use crate::error::SpiderError;
use crate::request::{Headers, Metadata, ProxyConfig, Request, RequestMode, SessionConfig};
use crate::validator::Validation;
use crate::value::Value;
use jiff::SignedDuration;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Config {
    pub r#type: String,
    pub options: BTreeMap<String, Value>,
}

impl Config {
    pub fn local(path: impl Into<String>) -> Self {
        let mut options = BTreeMap::new();
        options.insert("path".to_string(), Value::String(path.into()));
        Self {
            r#type: "local".to_string(),
            options,
        }
    }

    pub fn inline(value: Value) -> Self {
        let mut options = BTreeMap::new();
        options.insert("value".to_string(), value);
        Self {
            r#type: "inline".to_string(),
            options,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Dsl {
    pub steps: Vec<StepConfig>,
}

#[derive(Debug, Clone, Default)]
pub struct StepConfig {
    pub id: String,
    pub r#type: Option<String>, // "node" | "end"
    pub callback: Option<String>,
    pub fetch: FetchConfig,
    pub parse: ParseConfig,
    pub validate: Vec<Validation>,
    pub route: BTreeMap<String, Value>,
    pub output: BTreeMap<String, Value>,
    pub runtime: BTreeMap<String, Value>,
    pub middlewares: BTreeMap<String, Value>,
    pub meta: Option<BTreeMap<String, Value>>,
    pub dedup: Option<DedupConfig>,
    pub schedule: Option<ScheduleConfig>,
    pub retry: Option<RetryConfig>,
}

#[derive(Debug, Clone, Default)]
pub struct FetchConfig {
    pub mode: Option<String>,
    pub request: BTreeMap<String, Value>,
    pub browser: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub struct ParseConfig {
    pub fields: Vec<FieldConfig>,
    pub links: Vec<LinkConfig>,
    pub next_url_config: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct FieldConfig {
    pub name: String,
    pub source: String,
    pub selector_type: String,
    pub selector: Vec<String>,
    pub attribute: String,
    pub required: bool,
    pub default: Value,
    pub multiple: bool,
    pub options: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct LinkConfig {
    pub name: String,
    pub source: String,
    pub selector_type: String,
    pub selector: Vec<String>,
    pub attribute: String,
    pub required: bool,
    pub default: Value,
    pub multiple: bool,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub next_step: Option<String>,
    pub meta: BTreeMap<String, Value>,
    pub options: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Html,
    Text,
    Json,
    Xml,
    Headers,
    Url,
    Meta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorKind {
    Css,
    XPath,
    Json,
    Xml,
    Regex,
    Ai,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    pub steps: Vec<CompiledStep>,
}

impl Compiled {
    pub fn step_from_meta(&self, meta: &Metadata) -> Result<&CompiledStep, SpiderError> {
        let step_id = meta
            .get("next_step")
            .and_then(Value::as_str)
            .unwrap_or("parse");

        self.steps
            .iter()
            .find(|step| step.id == step_id)
            .ok_or_else(|| SpiderError::engine(format!("step not found: {step_id}")))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledStep {
    pub id: String,
    pub step_type: Option<String>,
    pub callback: Option<String>,
    pub fetch: FetchPlan,
    pub parse: ParsePlan,
    pub validate: Vec<Validation>,
    pub runtime: crate::runtime::Config,
    pub middlewares: crate::middleware::Map,
    pub meta: Option<BTreeMap<String, Value>>,
    pub dedup: Option<DedupConfig>,
    pub schedule: Option<ScheduleConfig>,
    pub retry: Option<RetryConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPlan {
    pub mode: Option<RequestMode>,
    pub method: Option<String>,
    pub headers: Headers,
    pub body: Option<Vec<u8>>,
    pub cookies: BTreeMap<String, String>,
    pub timeout: Option<SignedDuration>,
    pub proxy: Option<ProxyConfig>,
    pub session: Option<SessionConfig>,
    pub dont_filter: Option<bool>,
    pub http: Option<crate::request::http::Config>,
    pub browser: Option<crate::request::browser::Config>,
}

impl FetchPlan {
    pub fn apply_to_request(&self, mut request: Request) -> Request {
        if let Some(http) = &self.http {
            request = request.with_http(http.clone());
        } else if let Some(browser) = &self.browser {
            request = request.with_browser(browser.clone());
        } else if let Some(mode) = self.mode {
            request = request.with_mode(mode);
        }

        if let Some(method) = &self.method {
            request.method = method.clone();
        }

        for (key, values) in &self.headers {
            request.headers.insert(key.clone(), values.clone());
        }

        if let Some(body) = &self.body {
            request.body = Some(body.clone());
        }

        for (key, value) in &self.cookies {
            request.cookies.insert(key.clone(), value.clone());
        }

        if let Some(timeout) = self.timeout {
            request.timeout = Some(timeout);
        }

        if let Some(proxy) = &self.proxy {
            request.proxy = Some(proxy.clone());
        }

        if let Some(session) = &self.session {
            request.session = Some(session.clone());
        }

        if let Some(dont_filter) = self.dont_filter {
            request.dont_filter = dont_filter;
        }

        request
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsePlan {
    pub fields: Vec<FieldPlan>,
    pub links: Vec<LinkPlan>,
    pub next_url_config: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldPlan {
    pub name: String,
    pub source: SourceKind,
    pub source_ref: String,
    pub selector_type: SelectorKind,
    pub selector: Vec<String>,
    pub attribute: String,
    pub required: bool,
    pub default: Value,
    pub multiple: bool,
    pub options: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinkPlan {
    pub name: String,
    pub source: SourceKind,
    pub source_ref: String,
    pub selector_type: SelectorKind,
    pub selector: Vec<String>,
    pub attribute: String,
    pub required: bool,
    pub default: Value,
    pub multiple: bool,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub next_step: Option<String>,
    pub meta: BTreeMap<String, Value>,
    pub options: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DedupConfig {
    pub enabled: bool,
    pub key: Vec<String>,
    pub ttl: u64,
    pub scope: String,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScheduleConfig {
    pub concurrency: Option<u32>,
    pub interval: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RetryConfig {
    pub count: u32,
    pub http_status: Vec<u16>,
    pub backoff: Vec<u64>,
}
