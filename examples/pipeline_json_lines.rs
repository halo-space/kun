//! 文件 pipeline 示例：period.xml -> Spider item -> pipeline::JsonLines
//!
//! 展示：
//! - Spider 只负责产出 item
//! - 自定义 pipeline 在 `process()` 中补字段
//! - 内置 `pipeline::JsonLines` 把 item 逐行写入 JSON Lines 文件
//! - 引擎仍然只通过一条 pipeline 链路完成后处理与输出
//!
//! 运行：cargo run --example pipeline_json_lines

use halo_spider::download::{Browser, Http};
use halo_spider::engine::{Engine, ShutdownHandle};
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::pipeline::{JsonLines, Pipeline};
use halo_spider::response::Response;
use halo_spider::scheduler::Memory as SchedulerMemory;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use halo_spider::value::Value;
use jiff::SignedDuration;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct PeriodIssueSpider;

impl Spider for PeriodIssueSpider {
    fn name(&self) -> &str {
        "period_json_lines"
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

    let output_path = std::env::temp_dir().join("halo-spider-period-items.jsonl");
    let json_lines = JsonLines::new(output_path.clone());
    let settings = Settings::default().with_idle_timeout(SignedDuration::from_millis(200));

    let engine =
        Engine::new(SchedulerMemory::default(), Http::default(), Browser).with_settings(settings);
    let handle = engine.shutdown_handle();

    let mut engine = engine.with_pipeline((
        (EnrichIssue, json_lines.clone()),
        StopAfterFirst::new(handle),
    ));

    let outputs = engine.run(&PeriodIssueSpider).await?;
    let total_items = outputs
        .iter()
        .map(|output| output.items.len())
        .sum::<usize>();

    println!("engine returned {total_items} item(s)");
    println!("json lines written to: {}", display_path(&output_path));
    println!("{}", std::fs::read_to_string(json_lines.path())?);

    Ok(())
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn latest_issue(response: &Response) -> Result<(String, String), SpiderError> {
    let period_date = response
        .xml("//period[last()]/period_date")
        .text()
        .one()
        .ok_or_else(|| SpiderError::parse("未找到 period_date"))?;
    let front_page = response
        .xml("//period[last()]/front_page")
        .text()
        .one()
        .ok_or_else(|| SpiderError::parse("未找到 front_page"))?;

    Ok((period_date, front_page))
}

fn build_edition_url(period_date: &str, front_page: &str) -> Result<String, SpiderError> {
    let mut parts = period_date.split('-');
    let year = parts
        .next()
        .ok_or_else(|| SpiderError::parse("period_date 缺少年份"))?;
    let month = parts
        .next()
        .ok_or_else(|| SpiderError::parse("period_date 缺少月份"))?;
    let day = parts
        .next()
        .ok_or_else(|| SpiderError::parse("period_date 缺少日期"))?;

    Ok(format!(
        "https://ep.shxwcb.com/{year}/{month}/{day}/{front_page}?f={year}/{month}/period.xml"
    ))
}
