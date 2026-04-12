#![allow(refining_impl_trait)]

//! Custom dedup example.
//!
//! 展示：
//! - `Engine::new()` 默认还是精确 `dedup::Memory`
//! - 调用方可以实现 `dedup::Dedup` 并通过 `.with_dedup(...)` 替换
//! - 这个示例按 `method + url` 做最小自定义去重
//!
//! 运行：cargo run --example custom_dedup

use halo_spider::dedup::Dedup;
use halo_spider::engine::{Engine, ShutdownHandle};
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::pipeline::Pipeline;
use halo_spider::response::Response;
use halo_spider::settings::Config;
use halo_spider::spider::Spider;
use halo_spider::store::Memory as MemoryStore;
use halo_spider::value::Value;
use jiff::SignedDuration;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct MethodUrlDedup {
    seen: HashSet<String>,
}

impl MethodUrlDedup {
    fn new() -> Self {
        Self {
            seen: HashSet::new(),
        }
    }
}

impl Dedup for MethodUrlDedup {
    async fn check_and_insert(
        &mut self,
        request: &halo_spider::request::Request,
    ) -> Result<bool, SpiderError> {
        Ok(self
            .seen
            .insert(format!("{}|{}", request.method, request.url)))
    }
}

struct PeriodIssueSpider;

impl Spider for PeriodIssueSpider {
    fn name(&self) -> &str {
        "custom_dedup"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://ep.shxwcb.com/2026/03/period.xml".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Item, SpiderError> {
        let period_date = response
            .xml("//period[last()]/period_date")
            .text()
            .one()
            .ok_or_else(|| SpiderError::parse("period_date not found"))?;

        let item = Item::new()
            .with_field("period_date", Value::String(period_date))
            .with_field("dedup", Value::String("method+url".to_string()));

        Ok(item)
    }
}

#[derive(Clone)]
struct StopAfterFirst {
    handle: ShutdownHandle,
    stopped: Arc<AtomicBool>,
}

impl StopAfterFirst {
    fn new(handle: ShutdownHandle) -> Self {
        Self {
            handle,
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Pipeline for StopAfterFirst {
    async fn process(&self, _item: &mut Item, _spider_name: &str) -> Result<bool, SpiderError> {
        if !self.stopped.swap(true, Ordering::Relaxed) {
            self.handle.stop();
        }
        Ok(true)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    halo_spider::trace::init_console();

    let store = MemoryStore::default();
    let settings = Config::default().with_idle_timeout(SignedDuration::from_millis(200));

    let engine = Engine::new()
        .with_dedup(MethodUrlDedup::new())
        .with_config(settings);
    let handle = engine.shutdown_handle();

    let mut engine = engine
        .with_pipeline(StopAfterFirst::new(handle))
        .with_store(store.clone());

    engine.run(&PeriodIssueSpider).await?;

    println!("engine stored {} item(s)", engine.stats().item_count);
    for item in store.items() {
        println!("{item:#?}");
    }

    Ok(())
}
