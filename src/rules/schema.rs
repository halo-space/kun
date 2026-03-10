use crate::value::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Config {
    pub r#type: String,
    pub options: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub struct Dsl {
    pub steps: Vec<StepConfig>,
}

#[derive(Debug, Clone, Default)]
pub struct StepConfig {
    pub id: String,
    pub r#impl: String,
    pub callback: Option<String>,
    pub fetch: FetchConfig,
    pub parse: ParseConfig,
    pub route: BTreeMap<String, Value>,
    pub output: BTreeMap<String, Value>,
    pub runtime: BTreeMap<String, Value>,
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
    pub to: LinkTargetConfig,
    pub options: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub struct LinkTargetConfig {
    pub next_step: Option<String>,
    pub meta_patch: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepImpl {
    Dsl,
    Code,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Html,
    Text,
    Json,
    Xml,
    Headers,
    FinalUrl,
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
    Ocr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    pub steps: Vec<CompiledStep>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledStep {
    pub id: String,
    pub r#impl: StepImpl,
    pub callback: Option<String>,
    pub fetch: FetchPlan,
    pub parse: ParsePlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPlan {
    pub mode: crate::request::RequestMode,
    pub method: String,
    pub headers: crate::request::Headers,
    pub body: Option<Vec<u8>>,
    pub http: Option<crate::request::http::Config>,
    pub browser: Option<crate::request::browser::Config>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsePlan {
    pub fields: Vec<FieldPlan>,
    pub links: Vec<LinkPlan>,
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
    pub to: LinkTargetPlan,
    pub options: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinkTargetPlan {
    pub next_step: Option<String>,
    pub meta_patch: BTreeMap<String, Value>,
}
