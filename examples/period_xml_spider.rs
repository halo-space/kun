//! 三级爬取示例：period.xml -> 版面页 -> 文章页
//!
//! 展示代码爬虫的基础能力：
//! - XML 解析
//! - callback 链
//! - request.meta 透传
//! - response.follow 自动补全相对链接
//!
//! 运行：cargo run --example period_xml_spider

use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::response::Response;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use halo_spider::value::Value;
use halo_spider::{cb, spider_callbacks};
use jiff::SignedDuration;
use std::collections::{BTreeMap, BTreeSet};

struct PeriodSpider;

impl Spider for PeriodSpider {
    fn name(&self) -> &str {
        "period"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://ep.shxwcb.com/2026/03/period.xml".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
        let (period_date, front_page) = latest_issue(response)?;
        let list_url = build_edition_url(&period_date, &front_page)?;

        let mut meta = BTreeMap::new();
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

        let req = response
            .follow_with_meta(&list_url, &meta)
            .with_callback(cb!(Self::parse_edition));

        Ok(Output {
            items: vec![],
            requests: vec![req],
        })
    }

    spider_callbacks!(parse, parse_edition, parse_detail);
}

impl PeriodSpider {
    async fn parse_edition(&self, response: &Response) -> Result<Output, SpiderError> {
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
            "period.edition",
            vec![
                halo_spider::trace::prop("period_date", period_date),
                halo_spider::trace::prop("front_page", front_page),
                halo_spider::trace::prop("url", response.url.as_str()),
            ],
        );

        let links = extract_article_links(response);
        let mut requests = Vec::new();

        for link in links {
            let mut meta = response.meta.clone();
            meta.insert(
                "edition_url".to_string(),
                Value::String(response.url.clone()),
            );
            requests.push(
                response
                    .follow_with_meta(&link, &meta)
                    .with_callback(cb!(Self::parse_detail)),
            );
        }

        Ok(Output {
            items: vec![],
            requests,
        })
    }

    async fn parse_detail(&self, response: &Response) -> Result<Output, SpiderError> {
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

        let item = Item::new()
            .with_field("url", Value::String(response.url.clone()))
            .with_field("period_date", Value::String(period_date))
            .with_field("front_page", Value::String(front_page))
            .with_field("title", Value::String(title))
            .with_field("subtitle", Value::String(subtitle))
            .with_field("content_preview", Value::String(preview));

        Ok(Output {
            items: vec![item],
            requests: vec![],
        })
    }
}

#[tokio::main]
async fn main() {
    halo_spider::trace::init_console();

    let settings = Settings::default()
        .with_download_delay(SignedDuration::from_millis(0)) // 移除延迟，展示并发
        .with_concurrent_requests(16)
        .with_concurrent_requests_per_domain(8)
        .with_idle_timeout(SignedDuration::from_secs(10));

    let spider = PeriodSpider;
    let mut engine = Engine::new().with_settings(settings);

    match engine.run(&spider).await {
        Ok(_) => println!("\n=== Crawl Complete ==="),
        Err(e) => eprintln!("crawl failed: {e}"),
    }
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

fn extract_article_links(response: &Response) -> Vec<String> {
    let mut links = Vec::new();
    let mut seen = BTreeSet::new();

    for href in response.css("a").attr("href").all() {
        if !href.ends_with(".html") || !href.starts_with("20") {
            continue;
        }
        if seen.insert(href.clone()) {
            links.push(href);
        }
        if links.len() >= 3 {
            break;
        }
    }

    links
}

fn truncate(text: &str, max_chars: usize) -> String {
    let truncated: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        format!("{truncated}...")
    } else {
        truncated
    }
}
