//! 单 pipeline 链路示例：period.xml -> Spider item -> pipeline.process()
//!
//! 展示：
//! - Spider 只负责产出 item
//! - 自定义 pipeline 在 `process()` 中补字段
//! - 内置 `pipeline::Memory` 保存处理后的 item
//! - 引擎统一通过一条 item pipeline 完成后处理与保存
//!
//! 运行：cargo run --example pipeline_memory

use halo_spider::download::{Browser, Http};
use halo_spider::engine::{Engine, ShutdownHandle};
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::pipeline::{Memory as MemoryPipeline, Pipeline};
use halo_spider::response::Response;
use halo_spider::scheduler::Memory as SchedulerMemory;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use halo_spider::value::Value;
use jiff::SignedDuration;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct PeriodIssueSpider;

impl Spider for PeriodIssueSpider {
    fn name(&self) -> &str {
        "period_pipeline"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://ep.shxwcb.com/2026/03/period.xml".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
        let (period_date, front_page) = latest_issue(response)?;
        let edition_url = build_edition_url(&period_date, &front_page)?;

        let item = Item::new()
            .with_field("period_date", Value::String(period_date))
            .with_field("front_page", Value::String(front_page))
            .with_field("edition_url", Value::String(edition_url));

        Ok(Output {
            items: vec![item],
            requests: Vec::new(),
        })
    }
}

#[derive(Clone, Copy)]
struct EnrichIssue;

impl Pipeline for EnrichIssue {
    async fn process(&self, item: &mut Item, _spider_name: &str) -> Result<bool, SpiderError> {
        let period_date = item
            .get("period_date")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let front_page = item
            .get("front_page")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let issue_key = format!("{period_date}-front-{front_page}");

        item.insert("source", Value::String("period.xml".to_string()));
        item.insert("issue_key", Value::String(issue_key));

        Ok(true)
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
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    let stored = MemoryPipeline::default();
    let settings = Settings::default().with_idle_timeout(SignedDuration::from_millis(200));

    let engine =
        Engine::new(SchedulerMemory::default(), Http::default(), Browser).with_settings(settings);
    let handle = engine.shutdown_handle();

    // 处理顺序：
    // 1. EnrichIssue: 修改 item
    // 2. MemoryPipeline: 保存 item
    // 3. StopAfterFirst: 处理到首个 item 后停止引擎，方便示例自动退出
    let mut engine =
        engine.with_pipeline(((EnrichIssue, stored.clone()), StopAfterFirst::new(handle)));

    let outputs = engine.run(&PeriodIssueSpider).await?;
    let total_items = outputs
        .iter()
        .map(|output| output.items.len())
        .sum::<usize>();

    println!("engine returned {total_items} item(s)");
    println!("items stored by pipeline:");
    for item in stored.items() {
        println!("{item:#?}");
    }

    Ok(())
}

fn latest_issue(response: &Response) -> Result<(String, String), SpiderError> {
    let period_date = response
        .xml("//period[last()]/period_date")
        .text()
        .one()
        .ok_or_else(|| SpiderError::parse("period_date not found"))?;
    let front_page = response
        .xml("//period[last()]/front_page")
        .text()
        .one()
        .ok_or_else(|| SpiderError::parse("front_page not found"))?;

    Ok((period_date, front_page))
}

fn build_edition_url(period_date: &str, front_page: &str) -> Result<String, SpiderError> {
    let mut parts = period_date.split('-');
    let year = parts
        .next()
        .ok_or_else(|| SpiderError::parse("period_date is missing year"))?;
    let month = parts
        .next()
        .ok_or_else(|| SpiderError::parse("period_date is missing month"))?;
    let day = parts
        .next()
        .ok_or_else(|| SpiderError::parse("period_date is missing day"))?;

    Ok(format!(
        "https://ep.shxwcb.com/{year}/{month}/{day}/{front_page}?f={year}/{month}/period.xml"
    ))
}
