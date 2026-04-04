//! Kafka store example: period.xml -> Spider item -> store::Kafka
//!
//! Shows:
//! - Spider only produces items
//! - final item is assembled directly in `parse()`
//! - built-in `store::Kafka` keeps the same fixed `parse -> item -> pipeline -> store` chain
//! - Kafka is treated as another final store boundary, not a separate sink runtime
//!
//! Run:
//!   HALO_SPIDER_KAFKA_BROKERS=127.0.0.1:9092 cargo run --example kafka

use halo_spider::engine::{Engine, ShutdownHandle};
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::pipeline::Pipeline;
use halo_spider::response::Response;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use halo_spider::store::Kafka;
use halo_spider::value::Value;
use jiff::SignedDuration;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

struct PeriodIssueSpider;

impl Spider for PeriodIssueSpider {
    fn name(&self) -> &str {
        "period_kafka"
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

    let brokers =
        std::env::var("HALO_SPIDER_KAFKA_BROKERS").unwrap_or_else(|_| "127.0.0.1:9092".to_string());
    let topic =
        std::env::var("HALO_SPIDER_KAFKA_TOPIC").unwrap_or_else(|_| "period_items".to_string());
    let kafka = Kafka::new(&brokers, &topic);
    let settings = Settings::default().with_idle_timeout(SignedDuration::from_millis(200));

    let engine = Engine::new().with_settings(settings);
    let handle = engine.shutdown_handle();

    let mut engine = engine
        .with_pipeline(StopAfterFirst::new(handle))
        .with_store(kafka);

    let outputs = engine.run(&PeriodIssueSpider).await?;
    let total_items = outputs
        .iter()
        .map(|output| output.items.len())
        .sum::<usize>();

    println!("engine returned {total_items} item(s)");
    println!("sent item JSON message(s) to Kafka topic `{topic}` via `{brokers}`");

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
        "https://ep.shxwcb.com/{year}/{month}/{day}/page_{front_page}.html"
    ))
}
