use halo_spider::download::{Browser, Http};
use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::response::Response;
use halo_spider::scheduler::Memory;
use halo_spider::settings::Config;
use halo_spider::spider::{Output, Spider};
use halo_spider::value::Value;
use halo_spider::{cb, spider_callbacks};
use jiff::SignedDuration;
use std::collections::BTreeMap;

struct PeriodConcurrencySpider;

impl Spider for PeriodConcurrencySpider {
    fn name(&self) -> &str {
        "concurrency_control"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://ep.shxwcb.com/2026/03/period.xml".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
        let requests = recent_periods(response, 6)
            .into_iter()
            .map(|(period_date, front_page)| {
                let mut meta = BTreeMap::new();
                meta.insert(
                    "period_date".to_string(),
                    Value::String(period_date.clone()),
                );
                meta.insert("front_page".to_string(), Value::String(front_page.clone()));

                response
                    .follow_with_meta(build_edition_url(&period_date, &front_page), &meta)
                    .with_callback(cb!(Self::parse_edition))
            })
            .collect();

        Ok(Output {
            items: vec![],
            requests,
        })
    }

    spider_callbacks!(parse, parse_edition);
}

impl PeriodConcurrencySpider {
    async fn parse_edition(&self, response: &Response) -> Result<Output, SpiderError> {
        let period_date = response
            .meta
            .get("period_date")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let title = response.css("h2.S10_bb").text().one().unwrap_or_default();

        println!("Fetched edition: {} -> {}", period_date, response.url);

        Ok(Output {
            items: vec![
                Item::new()
                    .with_field("period_date", Value::String(period_date.to_string()))
                    .with_field("edition_title", Value::String(title))
                    .with_field("url", Value::String(response.url.clone())),
            ],
            requests: vec![],
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    halo_spider::trace::init_console();

    let settings = Config::default()
        .with_concurrent_requests(3)
        .with_concurrent_requests_per_domain(1)
        .with_download_delay(SignedDuration::from_millis(200));

    let scheduler = Memory::default();
    let http = Http::new().with_pool_size(50);
    let browser = Browser;

    let mut engine = Engine::from_parts(scheduler, http, browser).with_config(settings);

    engine.run(&PeriodConcurrencySpider).await?;

    Ok(())
}

fn recent_periods(response: &Response, limit: usize) -> Vec<(String, String)> {
    let period_dates = response.xml("//period/period_date").text().all();
    let front_pages = response.xml("//period/front_page").text().all();

    period_dates
        .into_iter()
        .zip(front_pages)
        .rev()
        .take(limit)
        .collect()
}

fn build_edition_url(period_date: &str, front_page: &str) -> String {
    let mut parts = period_date.split('-');
    let year = parts.next().unwrap_or_default();
    let month = parts.next().unwrap_or_default();
    let day = parts.next().unwrap_or_default();

    format!("https://ep.shxwcb.com/{year}/{month}/{day}/{front_page}?f={year}/{month}/period.xml")
}
