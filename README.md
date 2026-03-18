# halo-spider

一个快速、异步的 Rust 爬虫框架，设计灵感来自 Scrapy。

## 特性

- **Scrapy 风格 API** — Spider + parse 回调 + Pipeline，上手即用
- **DSL / 代码双模式** — JSON 规则驱动或代码回调，可混合使用
- **全异步** — 基于 tokio，原生 async trait，无 async_trait 宏
- **中间件链** — 内建 7 个中间件（重试、去重、限速、Cookie、代理等），支持自定义扩展
- **持久引擎** — 队列空不退出，持续等待新任务，仅 stop() 或 Ctrl+C 停止
- **插件系统** — plugins.toml 声明式加载，(kind, name) 唯一标识
- **浏览器支持** — HTTP / Browser 双模式请求，headless_chrome 可选

## 快速开始

```toml
[dependencies]
halo-spider = "0.0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal"] }
tracing-subscriber = "0.3"
```

```rust
use halo_spider::download::{BrowserDownloader, HttpDownloader};
use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::response::Response;
use halo_spider::scheduler::memory::MemoryScheduler;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use halo_spider::value::Value;
use halo_spider::{cb, spider_callbacks};
use std::time::Duration;

struct MySpider;

impl Spider for MySpider {
    fn name(&self) -> &str { "my_spider" }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
        let titles = response.css("div.quote span.text::text").all();
        let items = titles.into_iter().map(|t| {
            halo_spider::item::Item::new()
                .with_field("text", Value::String(t))
        }).collect();
        Ok(Output { items, requests: vec![] })
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    let settings = Settings::default()
        .download_delay(Duration::from_millis(200))
        .idle_timeout(Duration::from_secs(5));

    let mut engine = Engine::new(
        MemoryScheduler::default(),
        HttpDownloader::default(),
        BrowserDownloader::default(),
    ).with_settings(settings);

    let handle = engine.shutdown_handle();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        handle.stop();
    });

    engine.run(&MySpider).await.unwrap();
}
```

## 示例

```bash
# Scrapy 风格递归爬虫
cargo run --example quotes_code

# DSL 规则驱动爬虫
cargo run --example quotes_dsl

# 自定义中间件
cargo run --example custom_middleware
```

## 核心概念

| 概念 | 说明 |
|------|------|
| **Spider** | 定义 name、start_urls、parse 回调 |
| **Settings** | 引擎级配置：速率、重试、去重、超时 |
| **Engine** | 调度执行：取任务 → 中间件 → 下载 → 回调 → 产出 |
| **Pipeline** | Item 后处理：清洗、存储、日志 |
| **Middleware** | 请求/响应/异常拦截链 |
| **Rules DSL** | JSON 声明式解析规则 |

## 自定义中间件

```rust
use halo_spider::middleware::traits::Middleware;
use halo_spider::middleware::types::{MiddlewareConfig, MiddlewareType};

// 方式一：直接注册实例
engine.add_middleware("my_mw", config, Box::new(MyMiddleware));

// 方式二：注册工厂，通过 Settings 配置驱动
engine.register_middleware("my_mw", |options| {
    Ok(Box::new(MyMiddleware::new(options)))
});
```

## 设计文档

详细的架构设计见 [DESIGN.md](DESIGN.md)。

## License

MIT
