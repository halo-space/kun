//! DSL 爬取示例：period.xml -> 版面页 -> 详情页
//!
//! 运行：cargo run --example period_xml_dsl

use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::pipeline::Pipeline;
use halo_spider::rules::Config as RulesConfig;
use halo_spider::settings::Config;
use halo_spider::spider::Spider;
use halo_spider::store::Memory as MemoryStore;
use jiff::SignedDuration;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct PeriodDslSpider;

impl Spider for PeriodDslSpider {
    fn name(&self) -> &str {
        "period_xml_dsl"
    }

    fn rules(&self) -> Option<RulesConfig> {
        Some(RulesConfig::local(format!(
            "{}/examples/rules/period_xml_dsl.json",
            env!("CARGO_MANIFEST_DIR")
        )))
    }
}

#[derive(Clone)]
struct StopWhenQuiet {
    last_item_at: Arc<AtomicU64>,
    seen_any: Arc<AtomicBool>,
}

impl StopWhenQuiet {
    fn new(last_item_at: Arc<AtomicU64>, seen_any: Arc<AtomicBool>) -> Self {
        Self {
            last_item_at,
            seen_any,
        }
    }
}

impl Pipeline for StopWhenQuiet {
    async fn process(&self, _item: &mut Item, _spider_name: &str) -> Result<bool, SpiderError> {
        self.seen_any.store(true, Ordering::Relaxed);
        self.last_item_at.store(unix_now_secs(), Ordering::Relaxed);
        Ok(true)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    halo_spider::trace::init_console();

    let store = MemoryStore::default();
    let settings = Config::default()
        .with_download_delay(SignedDuration::from_millis(0))
        .with_concurrent_requests(8)
        .with_concurrent_requests_per_domain(4)
        .with_idle_timeout(SignedDuration::from_secs(2));

    let spider = PeriodDslSpider;
    let engine = Engine::new().with_config(settings);
    let handle = engine.shutdown_handle();
    let last_item_at = Arc::new(AtomicU64::new(unix_now_secs()));
    let seen_any = Arc::new(AtomicBool::new(false));

    tokio::spawn({
        let handle = handle.clone();
        let last_item_at = last_item_at.clone();
        let seen_any = seen_any.clone();
        async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if !seen_any.load(Ordering::Relaxed) {
                    continue;
                }

                let idle_for = unix_now_secs().saturating_sub(last_item_at.load(Ordering::Relaxed));
                if idle_for >= 3 {
                    handle.stop();
                    break;
                }
            }
        }
    });

    let mut engine = engine
        .with_pipeline(StopWhenQuiet::new(last_item_at, seen_any))
        .with_store(store.clone());

    engine.run(&spider).await?;

    println!("\n=== DSL Crawl Complete ===");
    println!("items stored in memory: {}", store.items().len());

    for item in store.items().iter().take(5) {
        println!("{item:#?}");
    }

    Ok(())
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
