//! File store example: period.xml -> Spider item -> store::File
//!
//! 展示：
//! - Spider 只负责产出 item
//! - 最终 item 直接在 `parse()` 里组装完整
//! - 需要跨请求透传上下文时，优先走 `request.meta`，见 `period_xml_spider.rs`
//! - 内置 `store::File` 仍然走同一条最终 store 边界
//! - 也可以显式选择 pretty file format，而不影响主链语义
//!
//! 运行：cargo run --example file

use halo_spider::engine::{Engine, ShutdownHandle};
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::pipeline::Pipeline;
use halo_spider::response::Response;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use halo_spider::store::{File as FileStore, FileFormat};
use halo_spider::value::Value;
use jiff::SignedDuration;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct PeriodIssueSpider;

impl Spider for PeriodIssueSpider {
    fn name(&self) -> &str {
        "period_file"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://ep.shxwcb.com/2026/03/period.xml".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
        let (period_date, front_page) = latest_issue(response)?;
        let edition_url = build_edition_url(&period_date, &front_page)?;
        let issue_key = format!("{period_date}-front-{front_page}");

        let item = Item::new()
            .with_field("period_date", Value::String(period_date))
            .with_field("front_page", Value::String(front_page))
            .with_field("edition_url", Value::String(edition_url))
            .with_field("source", Value::String("period.xml".to_string()))
            .with_field("issue_key", Value::String(issue_key));

        Ok(Output {
            items: vec![item],
            requests: Vec::new(),
        })
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

    let output_path = std::env::temp_dir().join("halo-spider-period-items.pretty.json");
    let file_store = FileStore::new(output_path.clone()).with_format(FileFormat::PrettyJsonBlocks);
    let settings = Settings::default().with_idle_timeout(SignedDuration::from_millis(200));

    let engine = Engine::new().with_settings(settings);
    let handle = engine.shutdown_handle();

    let mut engine = engine
        .with_pipeline(StopAfterFirst::new(handle))
        .with_store(file_store.clone());

    let outputs = engine.run(&PeriodIssueSpider).await?;
    let total_items = outputs
        .iter()
        .map(|output| output.items.len())
        .sum::<usize>();

    println!("engine returned {total_items} item(s)");
    println!("file store wrote output to: {}", display_path(&output_path));
    println!(
        "{}",
        std::fs::read_to_string(file_store.path_for("period_file"))?
    );

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
