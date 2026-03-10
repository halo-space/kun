use crate::parser::{AiQuery, CssQuery, JsonQuery, RegexQuery, XmlQuery, XPathQuery};
use crate::request::{Headers, Metadata, Request};
use crate::value::Value;
use std::net::IpAddr;

#[derive(Debug, Clone, Default)]
pub struct CertificateInfo {
    pub subject: Option<String>,
    pub issuer: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub url: String,
    pub status: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
    pub text: String,
    pub meta: Metadata,
    pub request: Option<Box<Request>>,
    pub flags: Vec<String>,
    pub certificate: Option<CertificateInfo>,
    pub ip_address: Option<IpAddr>,
    pub protocol: Option<String>,
}

impl Response {
    pub fn css(&self, selector: impl Into<String>) -> CssQuery {
        CssQuery::new(selector)
    }

    pub fn xpath(&self, selector: impl Into<String>) -> XPathQuery {
        XPathQuery::new(selector)
    }

    pub fn json(&self, selector: Option<impl Into<String>>) -> JsonQuery {
        JsonQuery::new(selector.map(Into::into))
    }

    pub fn xml(&self, selector: impl Into<String>) -> XmlQuery {
        XmlQuery::new(selector)
    }

    pub fn regex(&self, pattern: impl Into<String>) -> RegexQuery {
        RegexQuery::new(pattern, Some("text".to_string()))
    }

    pub fn ai(&self, prompt: impl Into<String>) -> AiQuery {
        AiQuery::new(prompt, Some("html".to_string()))
    }

    pub fn follow(&self, url: impl Into<String>) -> Request {
        let mut request = Request::new(url);
        request.meta = self.meta.clone();
        request
    }
}

impl Default for Response {
    fn default() -> Self {
        Self {
            url: String::new(),
            status: 200,
            headers: Headers::new(),
            body: Vec::new(),
            text: String::new(),
            meta: Metadata::new(),
            request: None,
            flags: Vec::new(),
            certificate: None,
            ip_address: None,
            protocol: None,
        }
    }
}

pub fn merge_meta(current: &Metadata, patch: &BTreeMap<String, Value>) -> Metadata {
    let mut merged = current.clone();
    for (key, value) in patch {
        merged.insert(key.clone(), value.clone());
    }
    merged
}

use std::collections::BTreeMap;
