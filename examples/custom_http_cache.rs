//! Custom HTTP cache backend example.
//!
//! 展示：
//! - 调用方可以自己实现 `middleware::http_cache::Cache`
//! - 通过 `HttpCache::with_cache(...)` 把自定义 backend 接进引擎
//! - 自定义 backend 仍然复用内置 `HttpCache` 的条件请求、`304` 回填和 `ttl / strategy`
//!
//! 运行：cargo run --example custom_http_cache

use halo_spider::download::Browser;
use halo_spider::download::traits::Downloader;
use halo_spider::engine::{Engine, ShutdownHandle};
use halo_spider::error::SpiderError;
use halo_spider::future::BoxFuture;
use halo_spider::item::Item;
use halo_spider::middleware::http_cache::{Cache, Entry, HttpCache, Strategy};
use halo_spider::middleware::{Config, Stage};
use halo_spider::pipeline::Pipeline;
use halo_spider::request::{Headers, Request};
use halo_spider::response::Response;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use halo_spider::store::Memory as MemoryStore;
use halo_spider::value::Value;
use jiff::SignedDuration;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct DemoCache {
    entries: tokio::sync::Mutex<BTreeMap<String, Entry>>,
}

impl Cache for DemoCache {
    fn load<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Entry>, SpiderError>> {
        Box::pin(async move {
            let entry = self.entries.lock().await.get(key).cloned();
            halo_spider::trace::info(
                "http_cache.load",
                vec![
                    halo_spider::trace::prop("key", key),
                    halo_spider::trace::prop("hit", entry.is_some()),
                ],
            );
            Ok(entry)
        })
    }

    fn save<'a>(&'a self, entry: &'a Entry) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            halo_spider::trace::info(
                "http_cache.save",
                vec![halo_spider::trace::prop("key", entry.key.as_str())],
            );
            self.entries
                .lock()
                .await
                .insert(entry.key.clone(), entry.clone());
            Ok(())
        })
    }

    fn remove<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async move {
            halo_spider::trace::info(
                "http_cache.remove",
                vec![halo_spider::trace::prop("key", key)],
            );
            self.entries.lock().await.remove(key);
            Ok(())
        })
    }
}

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
                ("ETag".to_string(), vec!["custom-v1".to_string()]),
                ("X-Fetch-Count".to_string(), vec![current.to_string()]),
            ]
            .into_iter()
            .collect(),
            b"custom-cache-body".to_vec(),
        ))
    }
}

struct CacheSpider;

impl Spider for CacheSpider {
    fn name(&self) -> &str {
        "custom_http_cache"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://example.com/custom-cache".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
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
                    .with_dont_filter(true)
                    .with_meta("round", Value::Number(2.0)),
            ]
        } else {
            Vec::new()
        };

        Ok(Output {
            items: vec![item],
            requests,
        })
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

    let settings = Settings::default().with_idle_timeout(SignedDuration::from_millis(200));
    let custom_http_cache = HttpCache::default()
        .with_cache(DemoCache::default())
        .with_strategy(Strategy::Response)
        .with_ttl(SignedDuration::from_hours(1));

    let engine = Engine::new()
        .with_http(ConditionalCacheHttp::default())
        .with_browser(Browser)
        .with_settings(settings)
        .add_middleware(
            "http_cache",
            Config {
                enabled: true,
                stage: Stage::Download,
                order: 110,
                options: BTreeMap::new(),
            },
            Box::new(custom_http_cache),
        );
    let handle = engine.shutdown_handle();

    let store = MemoryStore::default();
    let mut engine = engine
        .with_pipeline(StopAfter::new(handle, 2))
        .with_store(store.clone());

    engine.run(&CacheSpider).await?;

    println!("stored items:");
    for item in store.items() {
        println!("{item:#?}");
    }
    println!("stats: {:#?}", engine.stats());

    Ok(())
}

fn has_header(headers: &Headers, target: &str) -> bool {
    headers.keys().any(|name| name.eq_ignore_ascii_case(target))
}
