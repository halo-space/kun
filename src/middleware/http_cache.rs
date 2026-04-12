pub mod file;

use crate::engine::{context, flow};
use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::middleware::HTTP_CACHE;
use crate::middleware::traits::Middleware;
use crate::request::{Headers, Request, RequestMode};
use crate::response::{Response, certificate::CertificateInfo};
use crate::value::Value;
use jiff::{SignedDuration, Timestamp};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;

pub use file::File;

const CACHE_FLAG: &str = HTTP_CACHE;
const IF_NONE_MATCH: &str = "If-None-Match";
const IF_MODIFIED_SINCE: &str = "If-Modified-Since";
const ETAG: &str = "ETag";
const LAST_MODIFIED: &str = "Last-Modified";
const DEFAULT_TTL: u64 = 86_400_000;

pub trait Cache: Send + Sync {
    fn load<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Entry>, SpiderError>>;
    fn save<'a>(&'a self, entry: &'a Entry) -> BoxFuture<'a, Result<(), SpiderError>>;
    fn remove<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), SpiderError>>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Strategy {
    Validators,
    #[default]
    Response,
}

impl Strategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Validators => "validators",
            Self::Response => "response",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "validators" => Some(Self::Validators),
            "response" => Some(Self::Response),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub key: String,
    pub url: String,
    pub status: u16,
    pub headers: Headers,
    pub body: Option<Vec<u8>>,
    pub flags: Vec<String>,
    pub certificate: Option<CertificateInfo>,
    pub ip_address: Option<IpAddr>,
    pub protocol: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub stored_at: u64,
}

#[derive(Debug, Default)]
pub struct Memory {
    entries: tokio::sync::Mutex<BTreeMap<String, Entry>>,
}

pub struct HttpCache {
    cache: Arc<dyn Cache>,
    ttl: Option<u64>,
    strategy: Strategy,
}

impl std::fmt::Debug for HttpCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpCache")
            .field("ttl", &self.ttl)
            .field("strategy", &self.strategy)
            .finish_non_exhaustive()
    }
}

impl Default for HttpCache {
    fn default() -> Self {
        Self {
            cache: Arc::new(Memory::default()),
            ttl: Some(DEFAULT_TTL),
            strategy: Strategy::default(),
        }
    }
}

impl Cache for Memory {
    fn load<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Entry>, SpiderError>> {
        Box::pin(async move { Ok(self.entries.lock().await.get(key).cloned()) })
    }

    fn save<'a>(&'a self, entry: &'a Entry) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            self.entries
                .lock()
                .await
                .insert(entry.key.clone(), entry.clone());
            Ok(())
        })
    }

    fn remove<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            self.entries.lock().await.remove(key);
            Ok(())
        })
    }
}

impl HttpCache {
    pub fn new(options: &BTreeMap<String, Value>) -> Self {
        let mut cache = Self::default();
        cache.strategy = parse_strategy(options).unwrap_or_default();
        cache.ttl = parse_ttl(options);

        if options.get("backend").and_then(Value::as_str) == Some("file") {
            let path = options
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("output/http-cache.json");
            cache.cache = Arc::new(File::new(path));
        }

        cache
    }

    pub fn with_cache(mut self, cache: impl Cache + 'static) -> Self {
        self.cache = Arc::new(cache);
        self
    }

    pub fn with_ttl(mut self, ttl: SignedDuration) -> Self {
        self.ttl = Some(non_negative_milliseconds(ttl));
        self
    }

    pub fn without_ttl(mut self) -> Self {
        self.ttl = None;
        self
    }

    pub fn with_strategy(mut self, strategy: Strategy) -> Self {
        self.strategy = strategy;
        self
    }
}

impl Middleware for HttpCache {
    async fn before_download(
        &self,
        context: &mut context::Download,
    ) -> Result<flow::Download, SpiderError> {
        let Some(key) = cache_key(&context.request) else {
            return Ok(flow::Download::Continue);
        };

        let Some(entry) = self.cache.load(&key).await? else {
            record_http_cache_miss(context);
            return Ok(flow::Download::Continue);
        };

        if self.is_stale(entry.stored_at, current_time()) || !entry.has_validators() {
            self.cache.remove(&key).await?;
            record_http_cache_miss(context);
            return Ok(flow::Download::Continue);
        }

        if let Some(stats) = context.stats() {
            stats.record_http_cache_revalidate();
        }
        entry.apply_conditionals(&mut context.request);

        Ok(flow::Download::Continue)
    }

    async fn after_download(
        &self,
        context: &mut context::Download,
        response: &mut Response,
    ) -> Result<flow::Download, SpiderError> {
        let Some(key) = cache_key(&context.request) else {
            return Ok(flow::Download::Continue);
        };

        match response.status {
            200 => {
                if let Some(entry) =
                    Entry::from_response(key.clone(), response, self.strategy, current_time())
                {
                    self.cache.save(&entry).await?;
                    if let Some(stats) = context.stats() {
                        stats.record_http_cache_store();
                    }
                } else {
                    self.cache.remove(&key).await?;
                }
            }
            304 => {
                let Some(mut entry) = self.cache.load(&key).await? else {
                    return Ok(flow::Download::Continue);
                };

                if self.is_stale(entry.stored_at, current_time()) {
                    self.cache.remove(&key).await?;
                    return Ok(flow::Download::Continue);
                }

                entry.refresh_from_not_modified(response, current_time());
                self.cache.save(&entry).await?;

                if let Some(restored) = entry.restore_response(&context.request) {
                    if let Some(stats) = context.stats() {
                        stats.record_http_cache_hit();
                    }
                    *response = restored;
                }
            }
            _ => {}
        }

        Ok(flow::Download::Continue)
    }
}

impl HttpCache {
    fn is_stale(&self, stored_at: u64, current_time: u64) -> bool {
        let Some(ttl) = self.ttl else {
            return false;
        };

        current_time.saturating_sub(stored_at) >= ttl
    }
}

impl Entry {
    pub fn from_response(
        key: String,
        response: &Response,
        strategy: Strategy,
        stored_at: u64,
    ) -> Option<Self> {
        let etag = header_value(&response.headers, ETAG);
        let last_modified = header_value(&response.headers, LAST_MODIFIED);

        if etag.is_none() && last_modified.is_none() {
            return None;
        }

        Some(Self {
            key,
            url: response.url.clone(),
            status: response.status,
            headers: response.headers.clone(),
            body: match strategy {
                Strategy::Validators => None,
                Strategy::Response => Some(response.body.clone()),
            },
            flags: response.flags.clone(),
            certificate: response.certificate.clone(),
            ip_address: response.ip_address,
            protocol: response.protocol.clone(),
            etag,
            last_modified,
            stored_at,
        })
    }

    fn has_validators(&self) -> bool {
        self.etag.is_some() || self.last_modified.is_some()
    }

    fn apply_conditionals(&self, request: &mut Request) {
        if let Some(etag) = &self.etag {
            append_header_if_missing(&mut request.headers, IF_NONE_MATCH, etag);
        }

        if let Some(last_modified) = &self.last_modified {
            append_header_if_missing(&mut request.headers, IF_MODIFIED_SINCE, last_modified);
        }
    }

    fn restore_response(&self, request: &Request) -> Option<Response> {
        let body = self.body.clone()?;
        let mut response =
            Response::from_request(request.clone(), self.status, self.headers.clone(), body);
        response.url = self.url.clone();
        response.flags = self.flags.clone();
        if !response.flags.iter().any(|flag| flag == CACHE_FLAG) {
            response.flags.push(CACHE_FLAG.to_string());
        }
        response.certificate = self.certificate.clone();
        response.ip_address = self.ip_address;
        response.protocol = self.protocol.clone();
        Some(response)
    }

    fn refresh_from_not_modified(&mut self, response: &Response, stored_at: u64) {
        self.stored_at = stored_at;
        if let Some(etag) = header_value(&response.headers, ETAG) {
            self.etag = Some(etag);
        }
        if let Some(last_modified) = header_value(&response.headers, LAST_MODIFIED) {
            self.last_modified = Some(last_modified);
        }
    }
}

fn cache_key(request: &Request) -> Option<String> {
    if request.mode != RequestMode::Http || !request.method.eq_ignore_ascii_case("GET") {
        return None;
    }

    let mut url = Url::parse(&request.url).ok()?;

    if let Some(http) = &request.http
        && !http.query.is_empty()
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in &http.query {
            query.append_pair(key, value);
        }
    }

    Some(url.to_string())
}

fn append_header_if_missing(headers: &mut Headers, name: &str, value: &str) {
    if has_header(headers, name) {
        return;
    }

    headers
        .entry(name.to_string())
        .or_default()
        .push(value.to_string());
}

fn has_header(headers: &Headers, target: &str) -> bool {
    headers.keys().any(|name| name.eq_ignore_ascii_case(target))
}

fn header_value(headers: &Headers, target: &str) -> Option<String> {
    headers.iter().find_map(|(name, values)| {
        if name.eq_ignore_ascii_case(target) {
            values.last().cloned()
        } else {
            None
        }
    })
}

fn parse_strategy(options: &BTreeMap<String, Value>) -> Option<Strategy> {
    options
        .get("strategy")
        .and_then(Value::as_str)
        .and_then(Strategy::parse)
}

fn parse_ttl(options: &BTreeMap<String, Value>) -> Option<u64> {
    match options.get("ttl") {
        Some(Value::Null) => None,
        Some(Value::Number(value)) if value.is_finite() => Some((*value).max(0.0) as u64),
        _ => Some(DEFAULT_TTL),
    }
}

fn non_negative_milliseconds(duration: SignedDuration) -> u64 {
    let millis = duration.as_millis();
    if millis <= 0 {
        0
    } else {
        u64::try_from(millis).unwrap_or_default()
    }
}

fn current_time() -> u64 {
    u64::try_from(Timestamp::now().as_millisecond()).unwrap_or_default()
}

fn record_http_cache_miss(context: &context::Download) {
    if let Some(stats) = context.stats() {
        stats.record_http_cache_miss();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Request;
    use crate::stats::Tracker;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn http_cache_adds_if_none_match_after_caching_etag_response() {
        let middleware = HttpCache::default();
        let request = Request::new("https://example.com/feed");
        let mut first = context::Download::new(request.clone());
        let mut first_response = Response::from_request(
            request.clone(),
            200,
            [("ETag".to_string(), vec!["v1".to_string()])]
                .into_iter()
                .collect(),
            b"body".to_vec(),
        );

        block_on(middleware.after_download(&mut first, &mut first_response)).unwrap();

        let mut second = context::Download::new(request);
        block_on(middleware.before_download(&mut second)).unwrap();

        assert_eq!(
            second
                .request
                .headers
                .get(IF_NONE_MATCH)
                .and_then(|values: &Vec<String>| values.first())
                .map(String::as_str),
            Some("v1")
        );
    }

    #[test]
    fn http_cache_adds_if_modified_since_after_caching_last_modified_response() {
        let middleware = HttpCache::default();
        let request = Request::new("https://example.com/feed");
        let mut first = context::Download::new(request.clone());
        let mut first_response = Response::from_request(
            request.clone(),
            200,
            [(
                "Last-Modified".to_string(),
                vec!["Wed, 21 Oct 2015 07:28:00 GMT".to_string()],
            )]
            .into_iter()
            .collect(),
            b"body".to_vec(),
        );

        block_on(middleware.after_download(&mut first, &mut first_response)).unwrap();

        let mut second = context::Download::new(request);
        block_on(middleware.before_download(&mut second)).unwrap();

        assert_eq!(
            second
                .request
                .headers
                .get(IF_MODIFIED_SINCE)
                .and_then(|values: &Vec<String>| values.first())
                .map(String::as_str),
            Some("Wed, 21 Oct 2015 07:28:00 GMT")
        );
    }

    #[test]
    fn http_cache_restores_cached_response_when_server_returns_not_modified() {
        let middleware = HttpCache::default();
        let request = Request::new("https://example.com/feed");
        let mut first = context::Download::new(request.clone());
        let mut first_response = Response::from_request(
            request.clone(),
            200,
            [("ETag".to_string(), vec!["v1".to_string()])]
                .into_iter()
                .collect(),
            b"cached".to_vec(),
        );

        block_on(middleware.after_download(&mut first, &mut first_response)).unwrap();

        let mut second = context::Download::new(request.clone());
        let mut response = Response::from_request(request, 304, Headers::new(), Vec::new());

        block_on(middleware.after_download(&mut second, &mut response)).unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.text, "cached");
        assert!(response.flags.iter().any(|flag| flag == CACHE_FLAG));
    }

    #[test]
    fn http_cache_strategy_validators_keeps_304_without_cached_body() {
        let middleware = HttpCache::default().with_strategy(Strategy::Validators);
        let request = Request::new("https://example.com/feed");
        let mut first = context::Download::new(request.clone());
        let mut first_response = Response::from_request(
            request.clone(),
            200,
            [("ETag".to_string(), vec!["v1".to_string()])]
                .into_iter()
                .collect(),
            b"cached".to_vec(),
        );

        block_on(middleware.after_download(&mut first, &mut first_response)).unwrap();

        let mut second = context::Download::new(request.clone());
        let mut response = Response::from_request(request, 304, Headers::new(), Vec::new());

        block_on(middleware.after_download(&mut second, &mut response)).unwrap();

        assert_eq!(response.status, 304);
        assert!(response.text.is_empty());
        assert!(!response.flags.iter().any(|flag| flag == CACHE_FLAG));
    }

    #[test]
    fn http_cache_ttl_expiration_turns_entry_into_miss() {
        let middleware = HttpCache::default().with_ttl(SignedDuration::from_millis(0));
        let tracker = Arc::new(Tracker::default());
        let request = Request::new("https://example.com/feed");
        let mut first = context::Download::new(request.clone()).with_stats(tracker.clone());
        let mut first_response = Response::from_request(
            request.clone(),
            200,
            [("ETag".to_string(), vec!["v1".to_string()])]
                .into_iter()
                .collect(),
            b"body".to_vec(),
        );

        block_on(middleware.after_download(&mut first, &mut first_response)).unwrap();

        let mut second = context::Download::new(request).with_stats(tracker.clone());
        block_on(middleware.before_download(&mut second)).unwrap();

        assert!(!second.request.headers.contains_key(IF_NONE_MATCH));
        assert_eq!(tracker.snapshot().http_cache_miss_count, 1);
        assert_eq!(tracker.snapshot().http_cache_revalidate_count, 0);
    }

    #[test]
    fn http_cache_records_store_and_miss_stats() {
        let middleware = HttpCache::default();
        let tracker = Arc::new(Tracker::default());
        let request = Request::new("https://example.com/feed");

        let mut first = context::Download::new(request.clone()).with_stats(tracker.clone());
        block_on(middleware.before_download(&mut first)).unwrap();

        let mut second = context::Download::new(request.clone()).with_stats(tracker.clone());
        let mut second_response = Response::from_request(
            request,
            200,
            [("ETag".to_string(), vec!["v1".to_string()])]
                .into_iter()
                .collect(),
            b"body".to_vec(),
        );

        block_on(middleware.after_download(&mut second, &mut second_response)).unwrap();

        assert_eq!(tracker.snapshot().http_cache_miss_count, 1);
        assert_eq!(tracker.snapshot().http_cache_store_count, 1);
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWake));
        let mut future = Pin::from(Box::new(future));
        let mut context = Context::from_waker(&waker);

        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }
}
