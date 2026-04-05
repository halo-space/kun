pub mod certificate;
pub mod follow;

use crate::parser::{
    AiQuery, CssQuery, FeedQuery, JsonQuery, RegexQuery, SitemapQuery, XPathQuery, XmlQuery,
};
use crate::request::{Headers, Metadata, Request};
use certificate::CertificateInfo;
use chardetng::EncodingDetector;
use encoding_rs::Encoding;
use follow::build_follow_request;
use regex::Regex as PatternRegex;
use std::net::IpAddr;
use std::sync::OnceLock;

const DOCUMENT_ENCODING_SNIFF_LIMIT: usize = 4096;

fn decode_response_text(headers: &Headers, body: &[u8]) -> String {
    if body.is_empty() {
        return String::new();
    }

    let (encoding, bom_len) = response_text_encoding(headers, body);
    let (text, _, _) = encoding.decode(&body[bom_len..]);
    text.into_owned()
}

fn response_text_encoding(headers: &Headers, body: &[u8]) -> (&'static Encoding, usize) {
    if let Some((encoding, bom_len)) = Encoding::for_bom(body) {
        return (encoding, bom_len);
    }

    let declared_charset = charset_from_headers(headers).or_else(|| charset_from_document(body));

    if let Some(charset) = declared_charset
        && let Some(encoding) = Encoding::for_label(charset.as_bytes())
    {
        return (encoding, 0);
    }

    (apparent_encoding(body), 0)
}

fn apparent_encoding(body: &[u8]) -> &'static Encoding {
    let mut detector = EncodingDetector::new();
    detector.feed(body, true);
    detector.guess(None, true)
}

fn empty_metadata() -> &'static Metadata {
    static EMPTY_METADATA: OnceLock<Metadata> = OnceLock::new();
    EMPTY_METADATA.get_or_init(Metadata::new)
}

fn charset_from_headers(headers: &Headers) -> Option<String> {
    for (name, values) in headers {
        if !name.eq_ignore_ascii_case("content-type") {
            continue;
        }

        for value in values.iter().rev() {
            if let Some(charset) = extract_charset_parameter(value) {
                return Some(charset);
            }
        }
    }

    None
}

fn extract_charset_parameter(value: &str) -> Option<String> {
    for parameter in value.split(';').skip(1) {
        let (name, raw_value) = parameter.split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("charset") {
            continue;
        }

        let charset = raw_value.trim().trim_matches('"').trim_matches('\'').trim();
        if charset.is_empty() {
            return None;
        }

        return Some(charset.to_string());
    }

    None
}

fn charset_from_document(body: &[u8]) -> Option<String> {
    let prefix = document_encoding_prefix(body);

    capture_charset(xml_encoding_regex(), &prefix)
        .or_else(|| capture_charset(html_meta_charset_regex(), &prefix))
        .or_else(|| capture_charset(html_meta_content_type_regex(), &prefix))
}

fn document_encoding_prefix(body: &[u8]) -> String {
    body.iter()
        .take(DOCUMENT_ENCODING_SNIFF_LIMIT)
        .map(|byte| {
            if byte.is_ascii() {
                byte.to_ascii_lowercase() as char
            } else {
                ' '
            }
        })
        .collect()
}

fn capture_charset(regex: &PatternRegex, input: &str) -> Option<String> {
    regex
        .captures(input)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_string())
}

fn xml_encoding_regex() -> &'static PatternRegex {
    static XML_ENCODING_REGEX: OnceLock<PatternRegex> = OnceLock::new();

    XML_ENCODING_REGEX.get_or_init(|| {
        PatternRegex::new(r#"<\?xml[^>]+encoding\s*=\s*["']\s*([a-z0-9._-]+)"#)
            .expect("xml encoding regex should compile")
    })
}

fn html_meta_charset_regex() -> &'static PatternRegex {
    static HTML_META_CHARSET_REGEX: OnceLock<PatternRegex> = OnceLock::new();

    HTML_META_CHARSET_REGEX.get_or_init(|| {
        PatternRegex::new(r#"<meta[^>]+charset\s*=\s*["']?\s*([a-z0-9._-]+)"#)
            .expect("html meta charset regex should compile")
    })
}

fn html_meta_content_type_regex() -> &'static PatternRegex {
    static HTML_META_CONTENT_TYPE_REGEX: OnceLock<PatternRegex> = OnceLock::new();

    HTML_META_CONTENT_TYPE_REGEX.get_or_init(|| {
        PatternRegex::new(r#"<meta[^>]+content\s*=\s*["'][^"']*charset\s*=\s*([a-z0-9._-]+)"#)
            .expect("html meta content-type regex should compile")
    })
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
    pub fn new(url: impl Into<String>, status: u16, headers: Headers, body: Vec<u8>) -> Self {
        let text = decode_response_text(&headers, &body);

        Self {
            url: url.into(),
            status,
            headers,
            body,
            text,
            ..Self::default()
        }
    }

    pub fn from_request(request: Request, status: u16, headers: Headers, body: Vec<u8>) -> Self {
        let url = request.url.clone();
        let meta = request.meta.clone();
        let text = decode_response_text(&headers, &body);

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

    pub fn kwargs(&self) -> &Metadata {
        if let Some(request) = self.request.as_deref() {
            &request.kwargs
        } else {
            empty_metadata()
        }
    }

    pub fn kwarg(&self, key: &str) -> Option<&crate::value::Value> {
        self.kwargs().get(key)
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

    pub fn sitemap(&self) -> SitemapQuery {
        SitemapQuery::new(self.text.clone())
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

    pub fn follow_with_meta(&self, url: impl Into<String>, meta: &Metadata) -> Request {
        build_follow_request(self, url.into(), None, meta)
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
    use crate::request::{RequestMode, SessionConfig};
    use crate::value::Value;
    use encoding_rs::Encoding;
    use jiff::SignedDuration;

    fn encode_text(label: &str, text: &str) -> Vec<u8> {
        let encoding = Encoding::for_label(label.as_bytes()).expect("encoding should exist");
        let (bytes, _, _) = encoding.encode(text);
        bytes.into_owned()
    }

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
    fn response_text_is_decoded_from_content_type_charset() {
        let body = encode_text("gbk", "你好，kun");
        let mut headers = Headers::new();
        headers.insert(
            "Content-Type".to_string(),
            vec!["text/html; charset=gbk".to_string()],
        );

        let response = Response::new("https://example.com", 200, headers, body.clone());

        assert_eq!(response.body, body);
        assert_eq!(response.text, "你好，kun");
    }

    #[test]
    fn response_text_uses_document_declared_charset_when_header_is_missing() {
        let html = r#"<html><head><meta charset="gbk"></head><body>中文页面</body></html>"#;
        let body = encode_text("gbk", html);

        let response = Response::new("https://example.com", 200, Headers::new(), body.clone());

        assert_eq!(response.body, body);
        assert_eq!(response.text, html);
    }

    #[test]
    fn response_text_uses_apparent_encoding_when_no_charset_is_declared() {
        let text = "中文页面中文页面中文页面中文页面中文页面，kun 抓取测试。";
        let body = encode_text("gbk", text);

        let response = Response::new("https://example.com", 200, Headers::new(), body.clone());

        assert_eq!(response.body, body);
        assert_eq!(response.text, text);
    }

    #[test]
    fn response_text_strips_utf8_bom_when_decoding_body() {
        let body = vec![0xef, 0xbb, 0xbf, b'h', b'e', b'l', b'l', b'o'];

        let response = Response::new("https://example.com", 200, Headers::new(), body.clone());

        assert_eq!(response.body, body);
        assert_eq!(response.text, "hello");
    }

    #[test]
    fn follow_inherits_meta_and_parent_mode() {
        let request =
            Request::browser("https://example.com/list").with_meta("page", Value::Number(1.0));
        let response = Response::from_request(request, 200, Headers::new(), b"list".to_vec());

        let follow_request = response.follow("https://example.com/detail");

        assert_eq!(follow_request.mode, RequestMode::Browser);
        assert_eq!(follow_request.meta.get("page"), Some(&Value::Number(1.0)));
    }

    #[test]
    fn follow_inherits_core_request_semantics_and_resets_request_payload() {
        let request = Request::new("https://example.com/list?page=1")
            .with_method("POST")
            .with_body("payload")
            .with_header("authorization", "Bearer token")
            .with_cookie("sid", "cookie-1")
            .with_timeout(SignedDuration::from_secs(8))
            .with_proxy("http://127.0.0.1:8080")
            .with_session("session-a")
            .with_kwarg("page_size", Value::Number(50.0))
            .with_callback("parse_list")
            .with_errback("handle_error")
            .with_dont_filter(true)
            .with_meta("page", Value::Number(1.0));
        let response = Response::from_request(request, 200, Headers::new(), b"list".to_vec());

        let follow_request = response.follow("../detail/1.html");

        assert_eq!(follow_request.url, "https://example.com/detail/1.html");
        assert_eq!(follow_request.method, "GET");
        assert!(follow_request.body.is_none());
        assert_eq!(
            follow_request.headers.get("authorization"),
            Some(&vec!["Bearer token".to_string()])
        );
        assert_eq!(follow_request.timeout, Some(SignedDuration::from_secs(8)));
        assert_eq!(
            follow_request
                .proxy
                .as_ref()
                .map(|proxy| proxy.url.as_str()),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(
            follow_request.session,
            Some(SessionConfig::new("session-a"))
        );
        assert!(follow_request.cookies.get("sid") == Some(&"cookie-1".to_string()));
        assert!(
            follow_request
                .http
                .as_ref()
                .is_some_and(|http| http.query.is_empty())
        );
        assert!(follow_request.kwargs.is_empty());
        assert!(follow_request.callback.is_none());
        assert!(follow_request.errback.is_none());
        assert!(!follow_request.dont_filter);
        assert_eq!(follow_request.meta.get("page"), Some(&Value::Number(1.0)));
    }

    #[test]
    fn response_exposes_request_kwargs() {
        let request = Request::new("https://example.com/detail")
            .with_kwarg("edition", Value::String("night".to_string()));
        let response = Response::from_request(request, 200, Headers::new(), b"detail".to_vec());

        assert_eq!(
            response.kwarg("edition"),
            Some(&Value::String("night".to_string()))
        );
    }

    #[test]
    fn response_css_returns_css_query() {
        let response = Response::new(
            "https://example.com",
            200,
            Headers::new(),
            b"<h1>x</h1>".to_vec(),
        );
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
    fn response_xpath_extracts_from_malformed_html() {
        let response = Response::new(
            "https://example.com",
            200,
            Headers::new(),
            b"<html><body><div><h1>Hello</h1><a href='/next'>Next".to_vec(),
        );

        assert_eq!(
            response.xpath("//body/div/h1").text().one().as_deref(),
            Some("Hello")
        );
        assert_eq!(
            response.xpath("//body/div/a").attr("href").one().as_deref(),
            Some("/next")
        );
    }

    #[test]
    fn response_json_supports_optional_selector() {
        let response = Response::default();

        assert_eq!(response.json(None::<String>).value.source, "$");
        assert_eq!(response.json(Some("$.data.id")).value.source, "$.data.id");
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
