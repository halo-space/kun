//! 最小 middleware plugin 示例
//!
//! 这个示例只演示 plugin 当前已经落地的那部分能力：
//! - 用 `PluginManifest` 声明一个 `kind = "middleware"` 的插件
//! - 用 `PluginRegistry` 注册 manifest
//! - 用 `Settings::with_middleware(...)` 启用对应 key
//! - 用 `register_middleware(...)` 提供中间件 factory
//! - 用 `load_plugins(...)` 完成 engine 装配
//!
//! 如果你想看 `plugins.toml` 文件加载、override 规则和多插件组合，
//! 继续看 `plugins_demo.rs`。
//!
//! 运行：
//! cargo run --example middleware_plugin

use halo_spider::engine::Engine;
use halo_spider::engine::context::EngineContext;
use halo_spider::engine::flow::Flow;
use halo_spider::error::SpiderError;
use halo_spider::future::BoxFuture;
use halo_spider::item::Item;
use halo_spider::middleware::traits::Middleware;
use halo_spider::middleware::{Config, Stage};
use halo_spider::plugins::{PluginManifest, PluginRegistry};
use halo_spider::response::Response;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use halo_spider::value::Value;
use jiff::SignedDuration;
use std::collections::BTreeMap;

struct RequestStampMiddleware {
    label: String,
}

impl RequestStampMiddleware {
    fn new(options: &BTreeMap<String, Value>) -> Self {
        let label = options
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("plugin-demo")
            .to_string();
        Self { label }
    }
}

impl Middleware for RequestStampMiddleware {
    fn process_request<'a>(
        &'a self,
        context: &'a mut EngineContext,
    ) -> BoxFuture<'a, Result<Flow, SpiderError>> {
        Box::pin(async move {
            context
                .request
                .headers
                .insert("X-Plugin-Label".to_string(), vec![self.label.clone()]);

            halo_spider::trace::info(
                "middleware_plugin.request",
                vec![
                    halo_spider::trace::prop("plugin", "request_stamp"),
                    halo_spider::trace::prop("label", self.label.as_str()),
                    halo_spider::trace::prop("url", context.request.url.as_str()),
                ],
            );

            Ok(Flow::Continue)
        })
    }
}

struct MiddlewarePluginSpider;

impl Spider for MiddlewarePluginSpider {
    fn name(&self) -> &str {
        "middleware_plugin_demo"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://ep.shxwcb.com/2026/03/period.xml".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
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
        let plugin_label = response
            .request
            .as_ref()
            .and_then(|request| {
                request
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("X-Plugin-Label"))
                    .and_then(|(_, values)| values.last())
                    .cloned()
            })
            .unwrap_or_else(|| "missing".to_string());

        Ok(Output {
            items: vec![
                Item::new()
                    .with_field("period_date", Value::String(period_date))
                    .with_field("front_page", Value::String(front_page))
                    .with_field("plugin_label", Value::String(plugin_label))
                    .with_field("url", Value::String(response.url.clone())),
            ],
            requests: vec![],
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), SpiderError> {
    halo_spider::trace::init_console();

    let mut registry = PluginRegistry::new();
    registry.register(PluginManifest {
        name: "request_stamp".to_string(),
        kind: "middleware".to_string(),
        entry: "examples::middleware_plugin::RequestStampMiddleware".to_string(),
        r#override: false,
    })?;

    let settings = Settings::default()
        .with_download_delay(SignedDuration::from_millis(300))
        .with_idle_timeout(SignedDuration::from_secs(3))
        .with_middleware(
            "request_stamp",
            Config {
                enabled: true,
                stage: Stage::Download,
                order: 20,
                options: BTreeMap::from([(
                    "label".to_string(),
                    Value::String("plugin-demo".to_string()),
                )]),
            },
        );

    let mut engine = Engine::new()
        .with_settings(settings)
        .register_middleware("request_stamp", |options| {
            Ok(Box::new(RequestStampMiddleware::new(options)))
        })
        .load_plugins(&registry)?;

    let outputs = engine.run(&MiddlewarePluginSpider).await?;
    let item_count: usize = outputs.iter().map(|output| output.items.len()).sum();

    println!(
        "middleware plugin example finished: {} run(s), {} item(s)",
        outputs.len(),
        item_count
    );

    Ok(())
}
