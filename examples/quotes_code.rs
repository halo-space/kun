//! 递归爬取示例 —— 完整演示 Scrapy 风格的代码模式
//!
//! 对应 Python 的写法：
//!
//! ```python
//! class QuotesSpider(Spider):
//!     name = "quotes"
//!     allowed_domains = ["quotes.toscrape.com"]
//!     start_urls = ["https://quotes.toscrape.com/"]
//!     custom_settings = { 'ITEM_PIPELINES': { 'LogPipeline': 300 } }
//!
//!     async def parse(self, response):
//!         for quote in response.css("div.quote"):
//!             yield { "text": ..., "author": ..., "tags": ... }
//!             yield response.follow(author_url, callback=self.parse_author,
//!                                   meta={"author_name": author})
//!         next_page = response.css("li.next a::attr(href)").get()
//!         if next_page:
//!             yield response.follow(next_page, callback=self.parse)
//!
//!     async def parse_author(self, response):
//!         yield { "name": ..., "born_date": ..., "bio": ... }
//! ```
//!
//! 运行：cargo run --example quotes_code
//! 按 Ctrl+C 优雅退出（引擎会在当前轮次结束后关闭）

use spider::download::{BrowserDownloader, HttpDownloader};
use spider::engine::Engine;
use spider::error::SpiderError;
use spider::item::Item;
use spider::pipeline::Pipeline;
use spider::response::Response;
use spider::scheduler::memory::MemoryScheduler;
use spider::settings::Settings;
use spider::spider::{Output, Spider};
use spider::value::Value;
use spider::{cb, spider_callbacks};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

// ─── Spider 定义 ───────────────────────────────────────────
// Spider 只负责：name、start_urls、allowed_domains、parse 回调
// 不包含 runtime/retry/dedup 等配置（那些在 Settings 里）

struct QuotesSpider;

impl Spider for QuotesSpider {
    fn name(&self) -> &str {
        "quotes"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/".to_string()]
    }

    fn allowed_domains(&self) -> Vec<String> {
        vec!["quotes.toscrape.com".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
        let mut items = Vec::new();
        let mut requests = Vec::new();

        let texts = response.css("div.quote span.text::text").all();
        let authors = response.css("div.quote small.author::text").all();
        let author_links = response.css("div.quote span a").attr("href").all();
        let tags_list = response.css("div.quote div.tags a.tag::text").all();

        for (i, text) in texts.iter().enumerate() {
            let author = authors.get(i).cloned().unwrap_or_default();
            let tags = tags_list.clone();

            let item = Item::new()
                .with_field("text", Value::String(text.clone()))
                .with_field("author", Value::String(author.clone()))
                .with_field(
                    "tags",
                    Value::Array(tags.into_iter().map(Value::String).collect()),
                );
            items.push(item);

            if let Some(href) = author_links.get(i) {
                let mut meta = BTreeMap::new();
                meta.insert("author_name".to_string(), Value::String(author));
                let req = response
                    .follow_with_meta(href, &meta)
                    .with_callback(cb!(Self::parse_author));
                requests.push(req);
            }
        }

        if let Some(next_href) = response.css("li.next a").attr("href").one() {
            tracing::info!(next = next_href.as_str(), "发现下一页，递归跟进");
            let req = response
                .follow(&next_href)
                .with_callback(cb!(Self::parse));
            requests.push(req);
        }

        Ok(Output { items, requests })
    }

    spider_callbacks!(parse, parse_author);
}

impl QuotesSpider {
    async fn parse_author(&self, response: &Response) -> Result<Output, SpiderError> {
        let meta_name = response
            .meta
            .get("author_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let title = response
            .css("h3.author-title::text")
            .one()
            .unwrap_or_else(|| meta_name.to_string());

        let born_date = response
            .css("span.author-born-date::text")
            .one()
            .unwrap_or_default();

        let born_location = response
            .css("span.author-born-location::text")
            .one()
            .unwrap_or_default();

        let bio = response
            .css("div.author-description::text")
            .one()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let item = Item::new()
            .with_field("type", Value::String("author".to_string()))
            .with_field("name", Value::String(title))
            .with_field("born_date", Value::String(born_date))
            .with_field("born_location", Value::String(born_location))
            .with_field("bio", Value::String(if bio.len() > 100 {
                format!("{}...", &bio[..100])
            } else {
                bio
            }));

        Ok(Output {
            items: vec![item],
            requests: vec![],
        })
    }
}

// ─── Pipeline 定义 ─────────────────────────────────────────

struct LogPipeline {
    count: Arc<AtomicUsize>,
}

impl LogPipeline {
    fn new() -> Self {
        Self {
            count: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Pipeline for LogPipeline {
    async fn open(&self, spider_name: &str) -> Result<(), SpiderError> {
        tracing::info!(spider = spider_name, "[LogPipeline] 已启动");
        Ok(())
    }

    async fn process(&self, item: &mut Item, spider_name: &str) -> Result<bool, SpiderError> {
        let n = self.count.fetch_add(1, Ordering::Relaxed) + 1;

        let item_type = item
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("quote");

        match item_type {
            "author" => {
                let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                tracing::info!(spider = spider_name, n, name, "[LogPipeline] 作者");
            }
            _ => {
                let text = item
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(|s| if s.len() > 50 { &s[..50] } else { s })
                    .unwrap_or("?");
                let author = item.get("author").and_then(|v| v.as_str()).unwrap_or("?");
                tracing::info!(spider = spider_name, n, author, text, "[LogPipeline] 名言");
            }
        }

        Ok(true)
    }

    async fn close(&self, spider_name: &str) -> Result<(), SpiderError> {
        tracing::info!(
            spider = spider_name,
            total = self.count.load(Ordering::Relaxed),
            "[LogPipeline] 已关闭，共处理 items"
        );
        Ok(())
    }
}

// ─── 入口 ───────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    // Settings = Scrapy 的 settings.py
    let settings = Settings::default()
        .download_delay(Duration::from_millis(200))
        .retry_times(2)
        .retry_http_codes(vec![500, 503])
        .idle_timeout(Duration::from_secs(5));

    let spider = QuotesSpider;

    let engine = Engine::new(
        MemoryScheduler::default(),
        HttpDownloader::default(),
        BrowserDownloader::default(),
    )
    .with_settings(settings)
    .with_pipeline(LogPipeline::new());

    // 获取 shutdown handle，用于 Ctrl+C 优雅退出
    let handle = engine.shutdown_handle();

    // 监听 Ctrl+C
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("收到 Ctrl+C，停止引擎...");
        handle.stop();
    });

    let mut engine = engine;
    match engine.run(&spider).await {
        Ok(outputs) => {
            let total: usize = outputs.iter().map(|o| o.items.len()).sum();
            println!("\n=== 抓取完成 ===");
            println!("总共 {} 轮，{} 个 items", outputs.len(), total);
        }
        Err(e) => eprintln!("抓取失败: {e}"),
    }
}
