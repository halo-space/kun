use halo_spider::download::{Browser, Http};
use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::response::Response;
use halo_spider::scheduler::Memory;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use jiff::SignedDuration;

struct AiSpider;

impl Spider for AiSpider {
    fn name(&self) -> &str {
        "ai_extraction"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://ep.shxwcb.com/2026/03/period.xml".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
        let mut query = response
            .ai("Read this XML and return JSON with the latest period_date and front_page from the last <period> node.")
            .with_max_retries(3)
            .with_timeout(SignedDuration::from_secs(30));

        query.execute().await.map_err(SpiderError::parse)?;

        if let Some(result) = query.one() {
            println!("AI extracted: {}", result);
        }

        Ok(Output::empty())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let settings = Settings::default()
        .with_openai_api_key(std::env::var("OPENAI_API_KEY").ok().unwrap())
        .with_openai_model("gpt-4o-mini");

    let scheduler = Memory::default();
    let http = Http::new();
    let browser = Browser;

    let mut engine = Engine::from_parts(scheduler, http, browser).with_settings(settings);

    engine.run(&AiSpider).await?;

    Ok(())
}
