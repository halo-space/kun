#![allow(refining_impl_trait)]

//! 三级爬取示例：period.xml -> 版面列表页 -> 详情页
//!
//! 展示：
//! - 按当前时间动态生成 period.xml 请求
//! - 从 XML 中解析最新一期日期和 front_page
//! - 拼接版面列表页链接
//! - 从列表页继续派生文章详情页请求
//! - 使用 request.meta 在多跳请求间透传上下文
//!
//! 运行：cargo run --example period_xml_spider

use halo_spider::engine::{Engine, ShutdownHandle};
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::pipeline::Pipeline;
use halo_spider::response::Response;
use halo_spider::settings::Config;
use halo_spider::spider::Spider;
use halo_spider::store::Memory as MemoryStore;
use halo_spider::value::Value;
use halo_spider::{cb, spider_callbacks};
use jiff::{SignedDuration, Zoned};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct PeriodSpider {
    progress: Arc<CrawlProgress>,
}

impl PeriodSpider {
    fn new() -> Self {
        Self {
            progress: Arc::new(CrawlProgress::default()),
        }
    }
}

impl Spider for PeriodSpider {
    fn name(&self) -> &str {
        "period"
    }

    fn build_start_requests(&self) -> Vec<halo_spider::request::Request> {
        let period_xml_url = current_month_period_xml_url();

        halo_spider::trace::info(
            "period.start",
            vec![halo_spider::trace::prop(
                "period_xml_url",
                period_xml_url.as_str(),
            )],
        );
        println!("period.xml url: {period_xml_url}");

        vec![halo_spider::request::Request::new(period_xml_url)]
    }

    async fn parse(
        &self,
        response: &Response,
    ) -> Result<halo_spider::request::Request, SpiderError> {
        let (period_date, front_page) = latest_issue(response)?;
        let list_url = build_edition_url(&period_date, &front_page)?;

        let mut meta = BTreeMap::new();
        meta.insert(
            "period_xml_url".to_string(),
            Value::String(response.url.clone()),
        );
        meta.insert(
            "period_date".to_string(),
            Value::String(period_date.clone()),
        );
        meta.insert("front_page".to_string(), Value::String(front_page.clone()));

        halo_spider::trace::info(
            "period.latest",
            vec![
                halo_spider::trace::prop("period_date", period_date.as_str()),
                halo_spider::trace::prop("front_page", front_page.as_str()),
                halo_spider::trace::prop("list_url", list_url.as_str()),
            ],
        );
        println!("list page url: {list_url}");

        let req = halo_spider::request::Request::new(list_url)
            .with_callback(cb!(Self::parse_list))
            .with_meta_map(meta);

        Ok(req)
    }

    spider_callbacks!(parse_list, parse_edition, parse_detail);
}

impl PeriodSpider {
    async fn parse_list(
        &self,
        response: &Response,
    ) -> Result<Vec<halo_spider::request::Request>, SpiderError> {
        let period_date = response
            .meta
            .get("period_date")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let front_page = response
            .meta
            .get("front_page")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        halo_spider::trace::info(
            "period.list",
            vec![
                halo_spider::trace::prop("period_date", period_date),
                halo_spider::trace::prop("front_page", front_page),
                halo_spider::trace::prop("url", response.url.as_str()),
            ],
        );

        let edition_urls = extract_edition_urls(response);
        if edition_urls.is_empty() {
            return Err(SpiderError::parse("no edition links found on issue page"));
        }

        let mut requests = Vec::new();
        let current_page_url = canonical_issue_url(&response.url);

        let current_page_links = extract_article_links(response);
        self.progress.add_details(current_page_links.len());
        println!(
            "edition pages discovered: {}, current page articles: {}",
            edition_urls.len(),
            current_page_links.len()
        );

        for link in current_page_links {
            let mut meta = response.meta.clone();
            meta.insert("list_url".to_string(), Value::String(response.url.clone()));
            let req = halo_spider::request::Request::new(response.urljoin(&link))
                .with_callback(cb!(Self::parse_detail))
                .with_meta_map(meta);
            requests.push(req);
        }

        let mut remaining_edition_pages = 0usize;
        for edition_url in edition_urls {
            if edition_url == current_page_url {
                continue;
            }

            let mut meta = response.meta.clone();
            meta.insert(
                "edition_url".to_string(),
                Value::String(edition_url.clone()),
            );
            let req = halo_spider::request::Request::new(edition_url)
                .with_callback(cb!(Self::parse_edition))
                .with_meta_map(meta);
            requests.push(req);
            remaining_edition_pages += 1;
        }
        self.progress.set_pending_editions(remaining_edition_pages);

        Ok(requests)
    }

    async fn parse_edition(
        &self,
        response: &Response,
    ) -> Result<Vec<halo_spider::request::Request>, SpiderError> {
        let links = extract_article_links(response);
        self.progress.add_details(links.len());
        self.progress.finish_edition();

        halo_spider::trace::info(
            "period.edition",
            vec![
                halo_spider::trace::prop("url", response.url.as_str()),
                halo_spider::trace::prop("articles", links.len()),
            ],
        );
        println!(
            "edition page url: {} (articles: {})",
            response.url,
            links.len()
        );

        let mut requests = Vec::new();
        for link in links {
            let mut meta = response.meta.clone();
            meta.insert("list_url".to_string(), Value::String(response.url.clone()));
            let req = halo_spider::request::Request::new(response.urljoin(&link))
                .with_callback(cb!(Self::parse_detail))
                .with_meta_map(meta);
            requests.push(req);
        }

        Ok(requests)
    }

    async fn parse_detail(&self, response: &Response) -> Result<Item, SpiderError> {
        halo_spider::trace::info(
            "period.detail",
            vec![halo_spider::trace::prop("url", response.url.as_str())],
        );

        let title = response
            .css("p.title1")
            .text()
            .fallback(response.css("title").text())
            .one()
            .unwrap_or_default();
        let subtitle = response.css("p.title5").text().one().unwrap_or_default();
        let content = response
            .css("td.content_tt")
            .text()
            .join("")
            .normalize_whitespace()
            .one()
            .unwrap_or_default();
        let preview = truncate(&content, 120);
        let lead = response.css("p.title3").text().one().unwrap_or_default();
        let period_date = response
            .meta
            .get("period_date")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let front_page = response
            .meta
            .get("front_page")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let period_xml_url = response
            .meta
            .get("period_xml_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let list_url = response
            .meta
            .get("list_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let item = Item::new()
            .with_field("url", Value::String(response.url.clone()))
            .with_field("period_xml_url", Value::String(period_xml_url))
            .with_field("period_date", Value::String(period_date))
            .with_field("front_page", Value::String(front_page))
            .with_field("list_url", Value::String(list_url))
            .with_field("title", Value::String(title))
            .with_field("lead", Value::String(lead))
            .with_field("subtitle", Value::String(subtitle))
            .with_field("content", Value::String(content))
            .with_field("content_preview", Value::String(preview));

        Ok(item)
    }
}

#[derive(Default)]
struct CrawlProgress {
    pending_editions: AtomicUsize,
    pending_details: AtomicUsize,
    seen_items: AtomicUsize,
}

impl CrawlProgress {
    fn set_pending_editions(&self, count: usize) {
        self.pending_editions.store(count, Ordering::Relaxed);
    }

    fn finish_edition(&self) {
        self.pending_editions.fetch_sub(1, Ordering::Relaxed);
    }

    fn add_details(&self, count: usize) {
        self.pending_details.fetch_add(count, Ordering::Relaxed);
    }

    fn record_item(&self) -> usize {
        self.seen_items.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn is_complete(&self, seen_items: usize) -> bool {
        self.pending_editions.load(Ordering::Relaxed) == 0
            && seen_items >= self.pending_details.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
struct StopAfterCount {
    handle: ShutdownHandle,
    progress: Arc<CrawlProgress>,
}

impl StopAfterCount {
    fn new(handle: ShutdownHandle, progress: Arc<CrawlProgress>) -> Self {
        Self { handle, progress }
    }
}

impl Pipeline for StopAfterCount {
    async fn process(&self, _item: &mut Item, _spider_name: &str) -> Result<bool, SpiderError> {
        let current = self.progress.record_item();
        if self.progress.is_complete(current) {
            self.handle.stop();
        }

        Ok(true)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    halo_spider::trace::init_console();

    let store = MemoryStore::default();
    let settings = Config::default()
        .with_download_delay(SignedDuration::from_millis(0))
        .with_concurrent_requests(8)
        .with_concurrent_requests_per_domain(4)
        .with_idle_timeout(SignedDuration::from_secs(2));

    let spider = PeriodSpider::new();
    let engine = Engine::new().with_config(settings);
    let handle = engine.shutdown_handle();

    let mut engine = engine
        .with_pipeline(StopAfterCount::new(handle, spider.progress.clone()))
        .with_store(store.clone());

    engine.run(&spider).await?;
    let total_items = engine.stats().item_count;

    println!("\n=== Crawl Complete ===");
    println!("items stored in memory: {}", store.items().len());
    println!("items emitted by callbacks: {total_items}");

    for item in store.items() {
        println!("{item:#?}");
    }

    Ok(())
}

fn latest_issue(response: &Response) -> Result<(String, String), SpiderError> {
    let period_date = response
        .xml("//period/period_date")
        .text()
        .all()
        .into_iter()
        .last()
        .ok_or_else(|| SpiderError::parse("period_date not found"))?;
    let front_page = response
        .xml("//period/front_page")
        .text()
        .all()
        .into_iter()
        .last()
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

fn current_month_period_xml_url() -> String {
    let now = Zoned::now();
    let year = i32::from(now.year());
    let month = i32::from(now.month());

    format!("https://ep.shxwcb.com/{year:04}/{month:02}/period.xml")
}

fn extract_article_links(response: &Response) -> Vec<String> {
    let mut links = Vec::new();
    let mut seen = BTreeSet::new();

    for href in response.css("#artPList1 a").attr("href").all() {
        if !href.ends_with(".html") || !href.starts_with("20") {
            continue;
        }
        if seen.insert(href.clone()) {
            links.push(href);
        }
    }

    links
}

fn extract_edition_urls(response: &Response) -> Vec<String> {
    let mut urls = Vec::new();
    let mut seen = BTreeSet::new();

    for href in response.css("a").attr("href").all() {
        if !href.ends_with(".html") || !href.contains("__") {
            continue;
        }

        let absolute = canonical_issue_url(&response.urljoin(&href));
        if seen.insert(absolute.clone()) {
            urls.push(absolute);
        }
    }

    urls
}

fn truncate(text: &str, max_chars: usize) -> String {
    let truncated: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn canonical_issue_url(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or(url).to_string()
}
