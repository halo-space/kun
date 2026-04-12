#![allow(refining_impl_trait)]

use halo_spider::download::traits::Downloader;
use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::request::Request;
use halo_spider::response::Response;
use halo_spider::spider::Spider;
use halo_spider::store;
use halo_spider::telemetry;

#[derive(Clone, Default)]
struct StubDownloader;

impl Downloader for StubDownloader {
    async fn fetch(&self, request: &Request) -> Result<Response, SpiderError> {
        Ok(Response::from_request(
            request.clone(),
            200,
            Default::default(),
            br#"<html><body><h1>telemetry demo</h1></body></html>"#.to_vec(),
        ))
    }
}

struct DemoSpider;

impl Spider for DemoSpider {
    fn name(&self) -> &str {
        "telemetry_demo"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://example.com/telemetry".to_string()]
    }

    async fn parse(&self, _response: &Response) -> Result<(), SpiderError> {
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), SpiderError> {
    let collector = telemetry::Collector::default();
    let file = telemetry::File::new("output/telemetry-demo.jsonl")
        .map_err(|error| SpiderError::engine(format!("open telemetry file failed: {error}")))?;
    let prometheus = telemetry::Prometheus::default();
    let telemetry = telemetry::Fanout::new()
        .with_exporter(collector.clone())
        .with_exporter(file.clone())
        .with_exporter(prometheus.clone());

    let mut engine = Engine::with_downloaders(StubDownloader, StubDownloader)
        .with_store(store::Memory::default())
        .with_telemetry(telemetry);

    engine.run(&DemoSpider).await?;
    let snapshot = collector.snapshot();

    println!("items: {}", engine.stats().item_count);
    println!(
        "stats: request={} response={} scheduler_claim={}",
        snapshot.stats.request_count,
        snapshot.stats.response_count,
        snapshot.stats.scheduler_claim_count
    );
    println!(
        "scheduler metrics: claimed={} completed={} closed={}",
        snapshot.scheduler.totals.claimed_total,
        snapshot.scheduler.totals.completed_total,
        snapshot.scheduler.totals.closed_total
    );
    println!("recent telemetry events: {}", snapshot.recent_events.len());
    println!("telemetry file written to {}", file.path().display());
    println!("prometheus metrics:\n{}", prometheus.render());

    Ok(())
}
