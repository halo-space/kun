#![allow(refining_impl_trait)]

//! HTTP cache example.
//!
//! 展示：
//! - `Config::with_http_cache(true)` 开启缓存
//! - `with_http_cache_ttl(...)`、`with_http_cache_strategy(...)`
//! - `with_http_cache_file(...)` 使用内置 file backend
//! - 同一个 URL 第二次请求会自动补条件请求头，并在 `304` 时回填缓存 body
//! - `Engine::stats()` 里可以直接看到 `miss / store / revalidate / hit`
//!
//! 运行：cargo run --example http_cache

use halo_spider::download::Browser;
use halo_spider::download::traits::Downloader;
use halo_spider::engine::{Engine, ShutdownHandle};
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::middleware::DEDUP;
use halo_spider::middleware::http_cache::Strategy;
use halo_spider::pipeline::Pipeline;
use halo_spider::request::{Headers, Request};
use halo_spider::response::Response;
use halo_spider::settings::Config;
use halo_spider::spider::Spider;
use halo_spider::store::Memory as MemoryStore;
use halo_spider::value::Value;
use jiff::SignedDuration;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct ConditionalCacheHttp {
    fetch_count: AtomicUsize,
}

impl Downloader for ConditionalCacheHttp {
    async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
        let current = self.fetch_count.fetch_add(1, Ordering::Relaxed) + 1;

        if has_header(&request.headers, "If-None-Match") {
            return Ok(Response::from_request(
                request.clone(),
                304,
                Headers::new(),
                Vec::new(),
            ));
        }

        Ok(Response::from_request(
            request.clone(),
            200,
            [
                ("ETag".to_string(), vec!["demo-v1".to_string()]),
                (
                    "Last-Modified".to_string(),
                    vec!["Wed, 21 Oct 2015 07:28:00 GMT".to_string()],
                ),
                ("X-Fetch-Count".to_string(), vec![current.to_string()]),
            ]
            .into_iter()
            .collect(),
            b"cached-body".to_vec(),
        ))
    }
}

struct HttpCacheSpider;

impl Spider for HttpCacheSpider {
    fn name(&self) -> &str {
        "http_cache"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://example.com/cache-demo".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<(Item, Vec<Request>), SpiderError> {
        let round = response
            .meta
            .get("round")
            .and_then(Value::as_f64)
            .unwrap_or(1.0);

        let item = Item::new()
            .with_field("round", Value::Number(round))
            .with_field("status", Value::Number(response.status as f64))
            .with_field("text", Value::String(response.text.clone()))
            .with_field(
                "flags",
                Value::Array(response.flags.iter().cloned().map(Value::String).collect()),
            );

        let requests = if round < 2.0 {
            vec![
                response
                    .follow(response.url.clone())
                    .skip([DEDUP])
                    .with_meta("round", Value::Number(2.0)),
            ]
        } else {
            Vec::new()
        };

        Ok((item, requests))
    }
}

#[derive(Clone)]
struct StopAfter {
    handle: ShutdownHandle,
    remaining: Arc<AtomicUsize>,
}

impl StopAfter {
    fn new(handle: ShutdownHandle, count: usize) -> Self {
        Self {
            handle,
            remaining: Arc::new(AtomicUsize::new(count)),
        }
    }
}

impl Pipeline for StopAfter {
    async fn process(&self, _item: &mut Item, _spider_name: &str) -> Result<bool, SpiderError> {
        if self.remaining.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.handle.stop();
        }
        Ok(true)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    halo_spider::trace::init_console();

    let cache_path = unique_cache_path();
    let store = MemoryStore::default();
    let settings = Config::default()
        .with_http_cache(true)
        .with_http_cache_ttl(SignedDuration::from_hours(12))
        .with_http_cache_strategy(Strategy::Response)
        .with_http_cache_file(cache_path.display().to_string())
        .with_idle_timeout(SignedDuration::from_millis(200));

    let engine = Engine::new()
        .with_http(ConditionalCacheHttp::default())
        .with_browser(Browser)
        .with_config(settings);
    let handle = engine.shutdown_handle();

    let mut engine = engine
        .with_pipeline(StopAfter::new(handle, 2))
        .with_store(store.clone());

    engine.run(&HttpCacheSpider).await?;

    println!("http cache file: {}", cache_path.display());
    println!("stored items:");
    for item in store.items() {
        println!("{item:#?}");
    }
    println!("stats: {:#?}", engine.stats());

    tokio::fs::remove_file(cache_path).await.ok();
    Ok(())
}

fn has_header(headers: &Headers, target: &str) -> bool {
    headers.keys().any(|name| name.eq_ignore_ascii_case(target))
}

fn unique_cache_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "halo-spider-http-cache-example-{}.json",
        std::process::id()
    ))
}
