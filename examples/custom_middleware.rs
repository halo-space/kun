//! 自定义中间件示例
//!
//! 演示两种注册自定义中间件的方式：
//!
//! 1. `add_middleware()` — 直接注册中间件实例到引擎级链
//! 2. `register_middleware()` — 注册工厂函数，通过 Settings/MIDDLEWARES 配置驱动
//!
//! 本示例实现三个自定义中间件：
//! - `custom_ua`: 为所有请求注入自定义 User-Agent（方式1）
//! - `request_logger`: 打印每个请求的 URL 和 headers（方式1）
//! - `stats`: 统计请求/响应数量，通过工厂注册（方式2）
//!
//! 运行：cargo run --example custom_middleware
//! 按 Ctrl+C 优雅退出

use halo_spider::download::{Browser, Http};
use halo_spider::engine::Engine;
use halo_spider::engine::context::EngineContext;
use halo_spider::engine::types::Flow;
use halo_spider::error::SpiderError;
use halo_spider::future::BoxFuture;
use halo_spider::item::Item;
use halo_spider::middleware::traits::Middleware;
use halo_spider::middleware::types::{MiddlewareConfig, MiddlewareType};
use halo_spider::response::Response;
use halo_spider::scheduler::Memory;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use halo_spider::value::Value;
use halo_spider::{cb, spider_callbacks};
use jiff::SignedDuration;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

// ─── 自定义中间件 1：UserAgent 注入 ─────────────────────────

struct UserAgentMiddleware {
    ua: String,
}

impl UserAgentMiddleware {
    fn new(ua: impl Into<String>) -> Self {
        Self { ua: ua.into() }
    }
}

impl Middleware for UserAgentMiddleware {
    fn process_request<'a>(
        &'a self,
        context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            context
                .request
                .headers
                .entry("User-Agent".to_string())
                .or_insert_with(|| vec![self.ua.clone()]);
            Ok(Flow::Continue)
        })
    }
}

// ─── 自定义中间件 2：请求日志 ───────────────────────────────

struct RequestLoggerMiddleware;

impl Middleware for RequestLoggerMiddleware {
    fn process_request<'a>(
        &'a self,
        context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            tracing::info!(
                url = context.request.url.as_str(),
                method = context.request.method.as_str(),
                headers = ?context.request.headers,
                "[RequestLogger] 发送请求"
            );
            Ok(Flow::Continue)
        })
    }

    fn process_response<'a>(
        &'a self,
        context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            if let Some(ref resp) = context.response {
                tracing::info!(
                    url = context.request.url.as_str(),
                    status = resp.status,
                    body_len = resp.body.len(),
                    "[RequestLogger] 收到响应"
                );
            }
            Ok(Flow::Continue)
        })
    }
}

// ─── 自定义中间件 3：统计中间件（通过工厂注册）─────────────

struct StatsMiddleware {
    request_count: AtomicUsize,
    response_count: AtomicUsize,
    label: String,
}

impl StatsMiddleware {
    fn new(options: &BTreeMap<String, Value>) -> Self {
        let label = options
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_string();

        Self {
            request_count: AtomicUsize::new(0),
            response_count: AtomicUsize::new(0),
            label,
        }
    }
}

impl Middleware for StatsMiddleware {
    fn process_request<'a>(
        &'a self,
        _context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            let n = self.request_count.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::info!(label = self.label.as_str(), count = n, "[Stats] 请求计数");
            Ok(Flow::Continue)
        })
    }

    fn process_response<'a>(
        &'a self,
        _context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            let n = self.response_count.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::info!(label = self.label.as_str(), count = n, "[Stats] 响应计数");
            Ok(Flow::Continue)
        })
    }
}

// ─── Spider 定义 ───────────────────────────────────────────

struct PeriodDemoSpider;

impl Spider for PeriodDemoSpider {
    fn name(&self) -> &str {
        "period_middleware_demo"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://ep.shxwcb.com/2026/03/period.xml".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
        Ok(Output {
            items: vec![],
            requests: vec![
                response
                    .follow_with_meta(&latest_edition_url(response)?, &latest_meta(response)?)
                    .with_callback(cb!(Self::parse_edition)),
            ],
        })
    }

    spider_callbacks!(parse, parse_edition);
}

impl PeriodDemoSpider {
    async fn parse_edition(&self, response: &Response) -> Result<Output, SpiderError> {
        let period_date = response
            .meta
            .get("period_date")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let title = response.css("h2.S10_bb").text().one().unwrap_or_default();

        tracing::info!(period_date, title = title.as_str(), "解析版面页");

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

// ─── 入口 ───────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    let settings = Settings::default()
        .with_download_delay(SignedDuration::from_millis(500))
        .with_idle_timeout(SignedDuration::from_secs(3));

    // 方式2：通过 Settings 配置 stats 中间件
    // 只需要声明名字和 options，引擎通过工厂自动实例化
    let settings = settings.with_middleware(
        "stats",
        MiddlewareConfig {
            enabled: true,
            r#type: MiddlewareType::Download,
            order: 10,
            options: BTreeMap::from([("label".to_string(), Value::String("global".to_string()))]),
        },
    );

    let mut engine = Engine::new(Memory::default(), Http::default(), Browser)
        .with_settings(settings)
        // 方式1：直接注册中间件实例 —— UserAgent
        .add_middleware(
            "custom_ua",
            MiddlewareConfig {
                enabled: true,
                r#type: MiddlewareType::Download,
                order: 100,
                options: BTreeMap::new(),
            },
            Box::new(UserAgentMiddleware::new(
                "KunSpider/1.0 (custom middleware example)",
            )),
        )
        // 方式1：直接注册中间件实例 —— RequestLogger
        .add_middleware(
            "request_logger",
            MiddlewareConfig {
                enabled: true,
                r#type: MiddlewareType::Download,
                order: 200,
                options: BTreeMap::new(),
            },
            Box::new(RequestLoggerMiddleware),
        )
        // 方式2：注册工厂 —— stats 通过配置驱动实例化
        .register_middleware("stats", |options| {
            Ok(Box::new(StatsMiddleware::new(options)))
        });

    let spider = PeriodDemoSpider;

    println!("=== 自定义中间件示例 ===");
    println!("Spider: {}", spider.name());
    println!();
    println!("已注册的引擎级中间件：");
    println!("  - custom_ua (order=100): 注入自定义 User-Agent");
    println!("  - request_logger (order=200): 打印请求/响应日志");
    println!();
    println!("已注册的配置驱动中间件：");
    println!("  - stats (order=10): 统计请求/响应数量（通过 Settings 配置）");
    println!();
    println!("中间件执行顺序：stats(10) → custom_ua(100) → request_logger(200)");
    println!("按 Ctrl+C 停止");
    println!();

    let handle = engine.shutdown_handle();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("收到 Ctrl+C，停止引擎...");
        handle.stop();
    });

    match engine.run(&spider).await {
        Ok(outputs) => {
            let total: usize = outputs.iter().map(|o| o.items.len()).sum();
            println!("\n=== 完成 ===");
            println!("共 {} 轮，{} 个 items", outputs.len(), total);
        }
        Err(e) => eprintln!("出错: {e}"),
    }
}

fn latest_period(response: &Response) -> Result<(String, String), SpiderError> {
    let period_date = response
        .xml("//period[last()]/period_date")
        .text()
        .one()
        .ok_or_else(|| SpiderError::parse("未找到 period_date"))?;
    let front_page = response
        .xml("//period[last()]/front_page")
        .text()
        .one()
        .ok_or_else(|| SpiderError::parse("未找到 front_page"))?;
    Ok((period_date, front_page))
}

fn latest_meta(response: &Response) -> Result<BTreeMap<String, Value>, SpiderError> {
    let (period_date, front_page) = latest_period(response)?;
    Ok(BTreeMap::from([
        ("period_date".to_string(), Value::String(period_date)),
        ("front_page".to_string(), Value::String(front_page)),
    ]))
}

fn latest_edition_url(response: &Response) -> Result<String, SpiderError> {
    let (period_date, front_page) = latest_period(response)?;
    let mut parts = period_date.split('-');
    let year = parts
        .next()
        .ok_or_else(|| SpiderError::parse("period_date 缺少年份"))?;
    let month = parts
        .next()
        .ok_or_else(|| SpiderError::parse("period_date 缺少月份"))?;
    let day = parts
        .next()
        .ok_or_else(|| SpiderError::parse("period_date 缺少日期"))?;

    Ok(format!(
        "https://ep.shxwcb.com/{year}/{month}/{day}/{front_page}?f={year}/{month}/period.xml"
    ))
}
