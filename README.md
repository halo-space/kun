# halo-spider

一个受 Scrapy 启发的 Rust 异步爬虫框架，支持代码回调与 JSON DSL 混合抓取，并且从现在开始只使用 OPSX / OpenSpec 作为项目规范与变更工作流。

## 当前状态

- 库代码位于 `src/`
- 示例位于 `examples/`
- 当前规范源位于 `openspec/specs/`
- 后续需求、方案、任务统一从 `openspec/changes/` 发起
- `openspec init` 生成的协作入口位于 `.claude/commands/opsx/` 与 `.codex/skills/`

旧的设计稿、分轮任务稿和 `.cursor` 规则/技能已经移除，不再作为本项目的协作入口。

## 快速开始

```toml
[dependencies]
halo-spider = "0.0.4"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal"] }
tracing-subscriber = "0.3"
```

```rust
use halo_spider::download::{Browser, Http};
use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::response::Response;
use halo_spider::scheduler::Memory;
use halo_spider::settings::Settings;
use halo_spider::spider::{Output, Spider};
use halo_spider::value::Value;
use std::time::Duration;

struct MySpider;

impl Spider for MySpider {
    fn name(&self) -> &str {
        "my_spider"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://quotes.toscrape.com/".to_string()]
    }

    async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
        let items = response
            .css("div.quote span.text::text")
            .all()
            .into_iter()
            .map(|text| halo_spider::item::Item::new().with_field("text", Value::String(text)))
            .collect();

        Ok(Output {
            items,
            requests: vec![],
        })
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    let settings = Settings::default()
        .with_download_delay(Duration::from_millis(200))
        .with_idle_timeout(Duration::from_secs(5));

    let mut engine = Engine::new(
        Memory::default(),
        Http::default(),
        Browser::default(),
    )
    .with_settings(settings);

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
# 基础能力示例（统一使用 period.xml 场景）
cargo run --example period_xml_spider
cargo run --example custom_middleware
cargo run --example plugins_demo

# AI 选择器示例（需要 OPENAI_API_KEY 环境变量）
cargo run --example ai_extraction --features ai-selector

# 并发控制示例
cargo run --example concurrency_control
```

## DSL 编写流程（推荐）

使用 JSON DSL 规则文件驱动爬虫，无需编写解析代码：

```rust
use halo_spider::rules::Config as RulesConfig;
use halo_spider::spider::Spider;

struct MyDslSpider;

impl Spider for MyDslSpider {
    fn name(&self) -> &str {
        "my_dsl_spider"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://example.com".to_string()]
    }

    fn rules(&self) -> Option<RulesConfig> {
        Some(RulesConfig::local("path/to/rules.json"))
    }
}
```

引擎会自动加载、编译规则并分发到 DSL step。

当前 `examples/` 先优先展示 kun 现有的实际能力。DSL 配置示例会在配置面稳定后，再按模块补回。

DSL 不是另一套独立运行时，而是把代码爬虫已有能力配置化后的入口。无论是 Rust 回调模式还是 JSON DSL 模式，底层都走同一套框架执行链路：

```text
Spider / rules
    -> Request
    -> Engine
    -> Downloader
    -> Response
    -> callback 或 DSL step
    -> Output(items / requests)
    -> Engine 继续调度
```

这意味着：

- `Request` 是统一的执行单元，DSL 生成的请求和代码里手写的请求没有本质区别。
- `meta` 是请求级上下文参数，用来携带当前请求和后续链路需要透传的数据；它的角色类似 Scrapy 的 `Request.meta`。
- `dedup`、`schedule`、`retry` 等能力属于 `kun` 框架本身的爬虫能力，DSL 只是这些能力的配置化表达，不应实现成一套独立于代码爬虫的专用流程。

**高级用法：** 如需手动控制规则编译和应用，可使用 `compile_rules()` 和 `apply_dsl()`，但这不是推荐的入门路径。

**已知限制：**
- HTML 解析暂不支持 XPath 选择器（当前 XPath 实现基于 XML 解析器，对不规范 HTML 容错性差）
- 建议在 HTML 场景下使用 CSS 选择器替代 XPath
- 详见 [TODO.md](TODO.md)

### DSL 配置选项

**meta（透传字段）**：
```json
{
  "id": "parse_list",
  "meta": {
    "source": "homepage",
    "category": "news"
  }
}
```

`step.meta` 会在当前 step 生成后续 `Request` 时合并进 `request.meta`。它会和父请求已有的 `meta`、当前 step 已解析出的字段值、以及 `links[].meta` 一起继续往后传递。

如果只是某一条 link 需要给下一个请求单独补几个参数，用 `links[].meta`：

```json
{
  "next_step": "detail",
  "meta": {
    "from_list": true
  }
}
```

**dedup（去重配置）**：
```json
{
  "dedup": {
    "enabled": true,
    "key": ["product_id"],
    "ttl": 86400,
    "scope": "TASK"
  }
}
```

`dedup.key` 会基于统一的请求上下文工作：

- `url` 表示按请求 URL 去重
- `["product_id"]` 表示按 `request.meta.product_id` 去重
- `["product_id", "meta.category"]` 表示按多个参数拼接后的值去重
- `scope = TASK | STEP | CUSTOM` 会继续映射到共享 dedup 逻辑

**retry（重试配置）**：
```json
{
  "retry": {
    "count": 3,
    "http_status": [500, 502, 503],
    "backoff": [1000, 2000, 5000]
  }
}
```

`retry` 会编译为共享 runtime 的重试配置，由引擎在下载失败或命中指定 `http_status` 时统一处理。

**schedule（调度配置）**：
```json
{
  "schedule": {
    "concurrency": 2,
    "interval": 1000
  }
}
```

`schedule.interval` 和 `schedule.concurrency` 也会编译进共享 runtime/middleware 链路，因此 DSL 和代码爬虫遵守相同的节流与并发控制语义。

**next_url_config.FUNCTION（最小 URL 生成函数）**：
```json
{
  "next_url_config": {
    "mode": "FUNCTION",
    "fn": "concat",
    "args": [
      "https://ep.shxwcb.com/",
      {
        "fn": "replace",
        "args": [
          {"meta": "period_date"},
          "-",
          "/"
        ]
      },
      "/",
      {
        "fn": "coalesce",
        "args": [
          {"field": "front_page"},
          {"meta": "front_page"}
        ]
      }
    ]
  }
}
```

当前先只支持 3 个通用函数：

- `concat`：按顺序拼接所有参数
- `replace`：`replace(input, from, to)`
- `coalesce`：返回第一个非空值

`args` 里的参数目前支持这几类最小形态：

- 标量字面量：`"https://example.com/"`、`"-"`、`true`、`123`
- 字段引用：`{"field": "front_page"}`
- meta 引用：`{"meta": "period_date"}`
- 显式字面量：`{"value": "literal"}`
- 嵌套函数：`{"fn": "replace", "args": [...]}`

当前没有继续保留面向外部的 DSL 配置样例目录，避免在 DSL 结构调整期间给出过期示例。

## AI 选择器

使用 OpenAI API 进行智能内容提取：

```toml
[dependencies]
halo-spider = { version = "0.0.4", features = ["ai-selector"] }
```

```rust
// 设置 API key（优先从环境变量读取）
let settings = Settings::default()
    .with_openai_api_key(std::env::var("OPENAI_API_KEY").ok().unwrap())
    .with_openai_model("gpt-4o-mini");

// 使用自定义 API endpoint（兼容 OpenAI 的服务）
let settings = Settings::default()
    .with_openai_api_key("your-api-key")
    .with_openai_base_url("https://your-api-endpoint.com/v1")
    .with_openai_model("your-model-name");

// 在 parse 中使用，支持重试和超时配置
async fn parse(&self, response: &Response) -> Result<Output, SpiderError> {
    let mut query = response.ai("Extract the main article title and summary")
        .with_max_retries(3)
        .with_timeout(Duration::from_secs(30));
    query.execute().await.map_err(|e| SpiderError::parse(e))?;

    if let Some(result) = query.one() {
        println!("AI extracted: {}", result);
    }
    Ok(Output::empty())
}
```

**特性：**
- 自动重试机制（指数退避）
- 可配置超时时间
- 完善的错误处理

**注意：** AI 调用会产生 API 费用，建议仅在复杂内容提取场景使用。

## 并发控制配置

```rust
let settings = Settings::default()
    .with_concurrent_requests(16)              // 全局最大并发数
    .with_concurrent_requests_per_domain(8)    // 每个域名最大并发数
    .with_connection_pool_size(100)            // HTTP 连接池大小
    .with_download_delay(Duration::from_millis(200));  // 请求间延迟
```

参考 `examples/concurrency_control.rs` 或 `examples/README.md` 查看当前保留示例。

## 贡献指南

如果你想参与开发或了解项目的开发流程，请查看 [CONTRIBUTING.md](CONTRIBUTING.md)。
## License

MIT
