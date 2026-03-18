pub mod certificate;
pub mod follow;

use crate::parser::{AiQuery, CssQuery, FeedQuery, JsonQuery, RegexQuery, XmlQuery, XPathQuery};
use crate::request::{Headers, Metadata, Request};
use certificate::CertificateInfo;
use follow::build_follow_request;
use std::net::IpAddr;

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
    pub fn new(
        url: impl Into<String>,
        status: u16,
        headers: Headers,
        body: Vec<u8>,
    ) -> Self {
        let text = String::from_utf8_lossy(&body).into_owned();

        Self {
            url: url.into(),
            status,
            headers,
            body,
            text,
            ..Self::default()
        }
    }

    pub fn from_request(
        request: Request,
        status: u16,
        headers: Headers,
        body: Vec<u8>,
    ) -> Self {
        let url = request.url.clone();
        let meta = request.meta.clone();
        let text = String::from_utf8_lossy(&body).into_owned();

        Self {
            url,
            status,
            headers,
            body,
            text,
            meta,
            request: Some(Box::new(request)),
            ..Self::default()
        }
    }

    pub fn css(&self, selector: impl Into<String>) -> CssQuery {
        CssQuery::new(self.text.clone(), selector)
    }

    pub fn xpath(&self, selector: impl Into<String>) -> XPathQuery {
        XPathQuery::new(self.text.clone(), selector)
    }

    pub fn json(&self, selector: Option<impl Into<String>>) -> JsonQuery {
        JsonQuery::new(self.text.clone(), selector.map(Into::into))
    }

    pub fn xml(&self, selector: impl Into<String>) -> XmlQuery {
        XmlQuery::new(self.text.clone(), selector)
    }

    pub fn regex(&self, pattern: impl Into<String>) -> RegexQuery {
        RegexQuery::new(self.text.clone(), pattern, Some("text".to_string()))
    }

    pub fn ai(&self, prompt: impl Into<String>) -> AiQuery {
        AiQuery::new(self.text.clone(), prompt, Some("html".to_string()))
    }

    pub fn feed(&self) -> FeedQuery {
        FeedQuery::new(self.text.clone())
    }

    pub fn follow(&self, url: impl Into<String>) -> Request {
        self.follow_with_meta(url, &Metadata::new())
    }

    pub fn follow_with_callback(
        &self,
        url: impl Into<String>,
        callback: impl Into<String>,
    ) -> Request {
        build_follow_request(self, url.into(), Some(callback.into()), &Metadata::new())
    }

    pub fn follow_with_meta(&self, url: impl Into<String>, meta_patch: &Metadata) -> Request {
        build_follow_request(self, url.into(), None, meta_patch)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Kind;
    use crate::request::RequestMode;
    use crate::value::Value;

    #[test]
    fn response_default_has_all_core_fields() {
        let response = Response::default();

        assert_eq!(response.status, 200);
        assert!(response.body.is_empty());
        assert!(response.text.is_empty());
        assert!(response.meta.is_empty());
        assert!(response.request.is_none());
        assert!(response.flags.is_empty());
        assert!(response.certificate.is_none());
        assert!(response.ip_address.is_none());
        assert!(response.protocol.is_none());
    }

    #[test]
    fn response_from_request_inherits_meta() {
        let request = Request::browser("https://example.com/detail")
            .with_meta("from_list", Value::Bool(true));

        let response = Response::from_request(request, 200, Headers::new(), b"ok".to_vec());

        assert_eq!(response.meta.get("from_list"), Some(&Value::Bool(true)));
        assert_eq!(
            response.request.as_deref().map(|request| request.mode),
            Some(RequestMode::Browser)
        );
    }

    #[test]
    fn follow_inherits_meta_and_parent_mode() {
        let request = Request::browser("https://example.com/list")
            .with_meta("page", Value::Number(1.0));
        let response = Response::from_request(request, 200, Headers::new(), b"list".to_vec());

        let follow_request = response.follow("https://example.com/detail");

        assert_eq!(follow_request.mode, RequestMode::Browser);
        assert_eq!(follow_request.meta.get("page"), Some(&Value::Number(1.0)));
    }

    #[test]
    fn response_css_returns_css_query() {
        let response = Response::new("https://example.com", 200, Headers::new(), b"<h1>x</h1>".to_vec());
        let query = response.css("h1.title");

        assert_eq!(query.node.selector, "h1.title");
        assert_eq!(query.input, "<h1>x</h1>");
    }

    #[test]
    fn response_xpath_returns_xpath_query() {
        let response = Response::default();
        let query = response.xpath("//h1");

        assert_eq!(query.node.selector, "//h1");
        assert_eq!(query.input, response.text);
    }

    #[test]
    fn response_json_supports_optional_selector() {
        let response = Response::default();

        assert_eq!(response.json(None::<String>).value.source, "$");
        assert_eq!(
            response.json(Some("$.data.id")).value.source,
            "$.data.id"
        );
    }

    #[test]
    fn response_xml_returns_xml_query() {
        let response = Response::default();
        let query = response.xml("//item/title");

        assert_eq!(query.node.selector, "//item/title");
        assert_eq!(query.input, response.text);
    }

    #[test]
    fn response_regex_uses_text_value_query() {
        let response = Response::default();
        let query = response.regex(r"title:\s+(.*)");

        assert_eq!(query.source.as_deref(), Some("text"));
        assert_eq!(query.value.kind, Kind::RegexGroup);
    }

    #[test]
    fn response_ai_uses_html_value_query() {
        let response = Response::default();
        let query = response.ai("extract title");

        assert_eq!(query.source.as_deref(), Some("html"));
        assert_eq!(query.value.kind, Kind::Ai);
    }
}
