//! SQLite store example: period.xml -> Spider item -> store::Sqlite
//!
//! 展示：
//! - Spider 只负责产出 item
//! - 最终 item 直接在 `parse()` 里组装完整
//! - 需要跨请求透传上下文时，优先走 `request.meta`，见 `period_xml_spider.rs`
//! - 内置 `store::Sqlite` 把 item 写入 SQLite
//! - 字段映射使用显式列类型，而完整 item 仍会落到 `item_json`
//!
//! 运行：cargo run --example sqlite

use halo_spider::engine::{Engine, ShutdownHandle};
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::pipeline::Pipeline;
use halo_spider::response::Response;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use halo_spider::store::{FieldColumnType, Sqlite as SqliteStore};
use halo_spider::value::Value;
use jiff::SignedDuration;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct PeriodIssueSpider;

impl Spider for PeriodIssueSpider {
    fn name(&self) -> &str {
        "period_sqlite"
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

    let output_path = std::env::temp_dir().join("halo-spider-period-items.db");
    let sqlite = SqliteStore::new(output_path.clone())
        .with_table("period_items")
        .with_field_column("period_date", "period_date", FieldColumnType::Text)
        .with_field_column("front_page", "front_page", FieldColumnType::Text)
        .with_field_column("edition_url", "edition_url", FieldColumnType::Text)
        .with_field_column("issue_key", "issue_key", FieldColumnType::Text);
    let settings = Settings::default().with_idle_timeout(SignedDuration::from_millis(200));

    let engine = Engine::new().with_settings(settings);
    let handle = engine.shutdown_handle();

    let mut engine = engine
        .with_pipeline(StopAfterFirst::new(handle))
        .with_store(sqlite.clone());

    let outputs = engine.run(&PeriodIssueSpider).await?;
    let total_items = outputs
        .iter()
        .map(|output| output.items.len())
        .sum::<usize>();

    println!("engine returned {total_items} item(s)");
    println!("sqlite database written to: {}", display_path(&output_path));

    let pool = open_sqlite_pool(&output_path).await?;
    let row = sqlx::query(
        "SELECT period_date, front_page, edition_url, issue_key, item_json FROM period_items ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await?;

    let period_date: String = row.get("period_date");
    let front_page: String = row.get("front_page");
    let edition_url: String = row.get("edition_url");
    let issue_key: String = row.get("issue_key");
    let item_json: String = row.get("item_json");

    println!("latest row:");
    println!("  period_date = {period_date}");
    println!("  front_page = {front_page}");
    println!("  edition_url = {edition_url}");
    println!("  issue_key = {issue_key}");
    println!("  item_json = {item_json}");

    Ok(())
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

async fn open_sqlite_pool(path: &Path) -> Result<sqlx::SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .create_if_missing(true);

    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
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
