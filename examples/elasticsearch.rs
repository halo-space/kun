//! Custom Elasticsearch store example: period.xml -> Spider item -> custom Store
//!
//! Shows:
//! - users can implement `Store` directly instead of waiting for a built-in store
//! - the custom store still plugs into the same `parse -> item -> pipeline -> store` chain
//! - Elasticsearch can use `_doc` for single writes and `_bulk` for batch writes
//!
//! Run:
//!   HALO_SPIDER_ES_URL=http://127.0.0.1:9200 cargo run --example elasticsearch

use halo_spider::engine::{Engine, ShutdownHandle};
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::pipeline::Pipeline;
use halo_spider::response::Response;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use halo_spider::store::Store;
use halo_spider::value::Value;
use jiff::SignedDuration;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone)]
struct ElasticsearchStore {
    client: reqwest::Client,
    base_url: String,
    index: String,
}

impl ElasticsearchStore {
    fn new(base_url: impl Into<String>, index: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            index: index.into(),
        }
    }

    fn doc_url(&self) -> String {
        format!(
            "{}/{}/_doc",
            self.base_url.trim_end_matches('/'),
            self.index
        )
    }

    fn bulk_url(&self) -> String {
        format!(
            "{}/{}/_bulk",
            self.base_url.trim_end_matches('/'),
            self.index
        )
    }
}

impl Store for ElasticsearchStore {
    async fn write(&self, item: &Item, _spider_name: &str) -> Result<(), SpiderError> {
        let response = self
            .client
            .post(self.doc_url())
            .json(&item.to_json())
            .send()
            .await
            .map_err(|error| {
                SpiderError::engine(format!("elasticsearch store request failed: {error}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(SpiderError::engine(format!(
                "elasticsearch store returned {status}: {body}"
            )));
        }

        Ok(())
    }

    async fn batch_write(&self, items: &[Item], _spider_name: &str) -> Result<(), SpiderError> {
        if items.is_empty() {
            return Ok(());
        }

        let mut body = String::new();
        for item in items {
            body.push_str("{\"index\":{}}\n");
            body.push_str(&serde_json::to_string(&item.to_json()).map_err(|error| {
                SpiderError::engine(format!(
                    "failed to serialize item for elasticsearch store: {error}"
                ))
            })?);
            body.push('\n');
        }

        let response = self
            .client
            .post(self.bulk_url())
            .header("content-type", "application/x-ndjson")
            .body(body)
            .send()
            .await
            .map_err(|error| {
                SpiderError::engine(format!("elasticsearch bulk store request failed: {error}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(SpiderError::engine(format!(
                "elasticsearch bulk store returned {status}: {body}"
            )));
        }

        let payload: serde_json::Value = response.json().await.map_err(|error| {
            SpiderError::engine(format!(
                "failed to parse elasticsearch bulk store response: {error}"
            ))
        })?;

        if payload
            .get("errors")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            return Err(SpiderError::engine(format!(
                "elasticsearch bulk store reported item errors: {payload}"
            )));
        }

        Ok(())
    }
}

struct PeriodIssueSpider;

impl Spider for PeriodIssueSpider {
    fn name(&self) -> &str {
        "period_elasticsearch"
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
    halo_spider::trace::init_console();

    let base_url =
        std::env::var("HALO_SPIDER_ES_URL").unwrap_or_else(|_| "http://127.0.0.1:9200".into());
    let index =
        std::env::var("HALO_SPIDER_ES_INDEX").unwrap_or_else(|_| "period_items".to_string());
    let store = ElasticsearchStore::new(&base_url, &index);
    let settings = Settings::default().with_idle_timeout(SignedDuration::from_millis(200));

    let engine = Engine::new().with_settings(settings);
    let handle = engine.shutdown_handle();

    let mut engine = engine
        .with_pipeline(StopAfterFirst::new(handle))
        .with_store(store);

    let outputs = engine.run(&PeriodIssueSpider).await?;
    let total_items = outputs
        .iter()
        .map(|output| output.items.len())
        .sum::<usize>();

    println!("engine returned {total_items} item(s)");
    println!("sent item JSON to Elasticsearch index `{index}` via `{base_url}`");

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
