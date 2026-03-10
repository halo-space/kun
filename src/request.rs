use crate::value::Value;
use std::collections::BTreeMap;

pub type Metadata = BTreeMap<String, Value>;
pub type Headers = BTreeMap<String, Vec<String>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestMode {
    Http,
    Browser,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeOverride {
    pub values: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default)]
pub struct CallbackTarget {
    pub name: String,
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
}

impl Request {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            mode: RequestMode::Http,
            method: "GET".to_string(),
            headers: Headers::new(),
            body: None,
            meta: Metadata::new(),
            callback: None,
            dont_filter: false,
            runtime: None,
        }
    }
}
