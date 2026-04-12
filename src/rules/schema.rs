use crate::error::SpiderError;
use crate::middleware::Map as MiddlewareMap;
use crate::request::{Headers, Metadata, ProxyConfig, Request, RequestMode, SessionConfig};
use crate::validator::FieldValidator;
use crate::value::Value;
use jiff::SignedDuration;
use std::collections::BTreeMap;

pub type RegistryOptions = BTreeMap<String, Value>;
pub type NamedRegistry = BTreeMap<String, RegistryOptions>;

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
    pub spider: SpiderConfig,
    pub engine: EngineRegistryConfig,
    pub sinks: BTreeMap<String, SinkConfig>,
    pub seeds: Vec<SeedConfig>,
    pub steps: Vec<StepConfig>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpiderConfig {
    pub name: String,
    pub clock: ClockConfig,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClockConfig {
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineRegistryConfig {
    pub dedup: NamedRegistry,
    pub concurrency: NamedRegistry,
    pub interval: NamedRegistry,
    pub rate_limit: NamedRegistry,
    pub auto_throttle: NamedRegistry,
    pub retry_by_status: NamedRegistry,
    pub retry_by_error: NamedRegistry,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SinkConfig {
    pub kind: String,
    pub options: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub struct SeedConfig {
    pub id: String,
    pub request: RequestConfig,
    pub meta: BTreeMap<String, ValueExpr>,
    pub allow_url_pattern: Vec<String>,
    pub engine: EngineRefs,
    pub next_step: String,
}

#[derive(Debug, Clone, Default)]
pub struct StepConfig {
    pub id: String,
    pub callback: Option<String>,
    pub fields: BTreeMap<String, FieldConfig>,
    pub bind: BTreeMap<String, ValueExpr>,
    pub follow: Vec<FollowConfig>,
    pub output: Option<OutputConfig>,
}

#[derive(Debug, Clone, Default)]
pub struct FollowConfig {
    pub item: Option<String>,
    pub next_step: String,
    pub request: RequestConfig,
    pub meta: BTreeMap<String, ValueExpr>,
    pub allow_url_pattern: Vec<String>,
    pub engine: EngineRefs,
}

#[derive(Debug, Clone, Default)]
pub struct OutputConfig {
    pub item: BTreeMap<String, ValueExpr>,
    pub validator: Option<OutputValidatorConfig>,
    pub sinks: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct OutputValidatorConfig {
    pub required: Vec<String>,
    pub fields: BTreeMap<String, OutputFieldValidatorConfig>,
}

#[derive(Debug, Clone, Default)]
pub struct OutputFieldValidatorConfig {
    pub value_type: String,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub format: Option<String>,
    pub pattern: Option<String>,
    pub enum_values: Vec<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct EngineRefs {
    pub dedup: Option<String>,
    pub concurrency: Option<String>,
    pub interval: Option<String>,
    pub rate_limit: Option<String>,
    pub auto_throttle: Option<String>,
    pub retry_by_status: Option<String>,
    pub retry_by_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RequestConfig {
    pub mode: Option<String>,
    pub method: Option<String>,
    pub url: ValueExpr,
    pub query: BTreeMap<String, ValueExpr>,
    pub headers: BTreeMap<String, ValueExpr>,
    pub cookies: BTreeMap<String, ValueExpr>,
    pub timeout: Option<ValueExpr>,
    pub proxy: Option<ValueExpr>,
    pub session: Option<ValueExpr>,
    pub encoding: Option<ValueExpr>,
    pub priority: Option<ValueExpr>,
    pub flags: Vec<ValueExpr>,
    pub cb_kwargs: BTreeMap<String, ValueExpr>,
    pub errback: Option<String>,
    pub body: Option<BodyConfig>,
    pub allow_redirects: Option<bool>,
    pub skip: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BodyConfig {
    Json(BTreeMap<String, ValueExpr>),
    Form(BTreeMap<String, ValueExpr>),
    Raw(ValueExpr),
}

#[derive(Debug, Clone, Default)]
pub struct FieldConfig {
    pub selector: String,
    pub kind: ExtractKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ExtractKind {
    #[default]
    Text,
    Html,
    Attribute(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValueExpr {
    pub source: ValueSource,
    pub transforms: Vec<TransformConfig>,
    pub fallback: Option<Box<ValueExpr>>,
}

impl Default for ValueExpr {
    fn default() -> Self {
        Self::literal(Value::Null)
    }
}

impl ValueExpr {
    pub fn literal(value: Value) -> Self {
        Self {
            source: ValueSource::Literal(value),
            transforms: Vec::new(),
            fallback: None,
        }
    }

    pub fn from_ref(path: impl Into<String>) -> Self {
        Self {
            source: ValueSource::From(path.into()),
            transforms: Vec::new(),
            fallback: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ValueSource {
    Literal(Value),
    From(String),
    Template {
        template: String,
        vars: BTreeMap<String, ValueExpr>,
    },
    Selector {
        selector: String,
        kind: SelectorValueKind,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransformConfig {
    pub kind: String,
    pub options: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorValueKind {
    Text,
    Html,
    Attribute(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorKind {
    Css,
    XPath,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    pub spider: SpiderConfig,
    pub engine: EngineRegistryConfig,
    pub sinks: BTreeMap<String, SinkConfig>,
    pub seeds: Vec<CompiledSeed>,
    pub steps: Vec<CompiledStep>,
}

impl Compiled {
    pub fn step_from_meta(&self, meta: &Metadata) -> Result<&CompiledStep, SpiderError> {
        let requested = meta.get("next_step").and_then(Value::as_str);

        if let Some(step_id) = requested {
            return self
                .steps
                .iter()
                .find(|step| step.id == step_id)
                .ok_or_else(|| SpiderError::engine(format!("step not found: {step_id}")));
        }

        self.steps
            .first()
            .ok_or_else(|| SpiderError::engine("compiled rules contain no steps".to_string()))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledSeed {
    pub id: String,
    pub request: RequestPlan,
    pub meta: BTreeMap<String, ValueExpr>,
    pub allow_url_pattern: Vec<String>,
    pub middleware: MiddlewareMap,
    pub next_step: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledStep {
    pub id: String,
    pub callback: Option<String>,
    pub fetch: FetchPlan,
    pub fields: BTreeMap<String, FieldPlan>,
    pub bind: BTreeMap<String, ValueExpr>,
    pub follow: Vec<FollowPlan>,
    pub output: Option<OutputPlan>,
    pub default_middlewares: MiddlewareMap,
    pub middlewares: MiddlewareMap,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldPlan {
    pub name: String,
    pub selector: String,
    pub selector_kind: SelectorKind,
    pub kind: ExtractKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FollowPlan {
    pub item: Option<String>,
    pub item_selector_kind: Option<SelectorKind>,
    pub next_step: String,
    pub request: RequestPlan,
    pub meta: BTreeMap<String, ValueExpr>,
    pub allow_url_pattern: Vec<String>,
    pub middleware: MiddlewareMap,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputPlan {
    pub item: BTreeMap<String, ValueExpr>,
    pub validators: Vec<FieldValidator>,
    pub sinks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestPlan {
    pub mode: Option<RequestMode>,
    pub method: Option<String>,
    pub url: ValueExpr,
    pub query: BTreeMap<String, ValueExpr>,
    pub headers: BTreeMap<String, ValueExpr>,
    pub cookies: BTreeMap<String, ValueExpr>,
    pub timeout: Option<ValueExpr>,
    pub proxy: Option<ValueExpr>,
    pub session: Option<ValueExpr>,
    pub encoding: Option<ValueExpr>,
    pub priority: Option<ValueExpr>,
    pub flags: Vec<ValueExpr>,
    pub cb_kwargs: BTreeMap<String, ValueExpr>,
    pub errback: Option<String>,
    pub body: Option<BodyConfig>,
    pub allow_redirects: Option<bool>,
    pub skip: Vec<String>,
}

impl Default for RequestPlan {
    fn default() -> Self {
        Self {
            mode: None,
            method: None,
            url: ValueExpr::default(),
            query: BTreeMap::new(),
            headers: BTreeMap::new(),
            cookies: BTreeMap::new(),
            timeout: None,
            proxy: None,
            session: None,
            encoding: None,
            priority: None,
            flags: Vec::new(),
            cb_kwargs: BTreeMap::new(),
            errback: None,
            body: None,
            allow_redirects: None,
            skip: Vec::new(),
        }
    }
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
    pub http: Option<crate::request::http::Config>,
    pub browser: Option<crate::request::browser::Config>,
}

impl Default for FetchPlan {
    fn default() -> Self {
        Self {
            mode: None,
            method: None,
            headers: Headers::new(),
            body: None,
            cookies: BTreeMap::new(),
            timeout: None,
            proxy: None,
            session: None,
            http: None,
            browser: None,
        }
    }
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

        request
    }
}
