# halo-spider

一个受 Scrapy 启发的 Rust 异步爬虫框架。当前优先补齐和稳定代码爬虫与共享底层能力；DSL 入口仍保留，但配置面暂时后置，并使用 OpenSpec 管理规范与变更。

## 当前状态

- 库代码位于 `src/`
- 示例位于 `examples/`
- 功能说明文档位于 `docs/capabilities.md`
- 当前规范源位于 `openspec/specs/`
- 后续需求、方案、任务统一从 `openspec/changes/` 发起
- `openspec init` 生成的协作入口位于 `.claude/commands/opsx/` 与 `.codex/skills/`

## 功能说明

按功能模块整理的能力说明见 [docs/capabilities.md](/Users/xiaohan/soft/project/xiaohan/kun/docs/capabilities.md)。

这份文档会集中解释：

- `Request`、`Response`、`download`、`scheduler`、`pipeline` 各自负责什么
- `scheduler::state::{Snapshot, Counts, Store}` 这组类型分别表示什么
- 当前已经落地的底层能力、明确的边界以及暂缓项

## 底层能力概览

README 这里只保留总览；模块级细节统一放到 [docs/capabilities.md](/Users/xiaohan/soft/project/xiaohan/kun/docs/capabilities.md)。

当前已落地的底层能力：

- `Request` 已经是统一执行单元，覆盖 `method`、`headers`、`body`、`timeout`、`proxy`、`session`、request cookies 与 `follow` 继承语义
- `download::Http` 已接到真实的 timeout、proxy、redirect、cookie jar 与 session cookies 能力
- `download::Browser` 已具备最小可用浏览器下载能力，并支持统一 `Request` 上的 `method` / `body` / `headers` / `timeout` / `proxy` / cookies / session
- `Response.body` 与 `Response.text` 的语义已经明确并统一解码
- `scheduler::Memory` 已把任务状态收口为 `ready / delayed / inflight`，并支持导出/恢复 `scheduler::state::Snapshot`
- `pipeline` 是唯一 item 处理链路，当前内置 `pipeline::Memory` 与 `pipeline::JsonLines`
- plugin 自动装载当前只支持 `middleware` kind，其它 kind 先保留命名空间
- DSL 当前定位已经明确为“共享底层能力的配置化入口”，不是另一套独立运行时

当前仍待补齐的底层能力：

- 共享 validation 还没有扩到更完整的规则集与失败策略
- 还没有内置 crash-safe durable scheduler 实现；当前只把 `scheduler::state::Store` 这层持久化边界显式建模出来
- 文件之外的数据库、消息队列等 pipeline 输出还没有内置实现
- HTML XPath、OCR、parse 后处理等 parser 能力仍未收敛

## 快速开始

```toml
[dependencies]
halo-spider = "0.0.5"
jiff = "0.2"
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
use jiff::SignedDuration;

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
        .with_download_delay(SignedDuration::from_millis(200))
        .with_idle_timeout(SignedDuration::from_secs(5));

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
cargo run --example pipeline_memory
cargo run --example pipeline_json_lines
cargo run --example custom_middleware
cargo run --example plugins_demo

# AI 选择器示例（需要 OPENAI_API_KEY 环境变量）
cargo run --example ai_extraction --features ai-selector

# 并发控制示例
cargo run --example concurrency_control
```

## Pipeline 组合

`pipeline` 是唯一的 item 处理链路。`with_pipeline((A, B))` 表示把两个
pipeline 串起来执行：

```rust
use halo_spider::pipeline::{Memory as MemoryPipeline, Pipeline};

#[derive(Clone, Copy)]
struct EnrichIssue;

impl Pipeline for EnrichIssue {
    async fn process(
        &self,
        item: &mut halo_spider::item::Item,
        _spider_name: &str,
    ) -> Result<bool, halo_spider::error::SpiderError> {
        item.insert(
            "source",
            halo_spider::value::Value::String("period.xml".to_string()),
        );
        Ok(true)
    }
}

let stored = MemoryPipeline::default();

let mut engine = Engine::new(scheduler, http, browser)
    .with_pipeline((EnrichIssue, stored.clone()));
```

执行顺序就是：

- `open()`：先 `A.open()`，再 `B.open()`
- `process()`：先 `A.process()`，只有当 `A` 返回 `Ok(true)` 时才继续执行 `B.process()`
- `close()`：先 `A.close()`，再 `B.close()`

也就是说，`with_pipeline((A, B))` 更像 `A -> B` 这条固定链路，而不是两条独立输出通道。
如果需要三个阶段，就继续嵌套元组：`((A, B), C)`。

完整可运行示例见 `examples/pipeline_memory.rs` 与 `examples/pipeline_json_lines.rs`。

## DSL 状态（暂缓）

项目里仍保留 `Spider::rules()` 这条入口，引擎也会继续负责加载和编译 rules：

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

但当前不把 DSL 作为主推使用方式，`examples/` 也不再维护字段级 DSL 示例。原因很简单：

- 当前优先把代码爬虫与共享底层能力做实
- DSL 只是这些底层能力的配置化入口，不是另一套独立运行时
- 在配置面重新收敛前，不继续维护容易过期的 DSL 字段清单和外部示例

DSL 当前的定位可以先简单理解成：把代码爬虫已有底层能力做成配置化入口。它不是另一套独立运行时。无论是 Rust 回调模式还是 JSON DSL 模式，底层都走同一套框架执行链路：

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
- `Response.body` 保留原始响应字节，`Response.text` 是从 `body` 派生出的解码文本；当前优先使用 BOM、`Content-Type charset` 与文档内编码声明，再回退 UTF-8 lossy。
- `dedup`、`schedule`、`retry` 等能力属于 `kun` 框架本身的爬虫能力，DSL 只是这些能力的配置化表达，不应实现成一套独立于代码爬虫的专用流程。

如果你现在要写新的爬虫，优先建议直接用代码模式；等共享底层能力进一步稳定后，再回到 DSL 配置面做统一收口。

## Browser 能力边界

当前 `browser` 模式走 `playwright-rs` 这条实现线，对外仍然只是 `kun` 的一个浏览器下载能力，不额外暴露单独的 backend 概念。

当前已经接线的最小能力：

- `engine = chromium | firefox | webkit`
- `headless`
- `viewport`
- `wait_for`
- request method
- request body
- request cookies
- request timeout
- request headers
- request proxy
- request session
- built-in `fingerprint_profile = desktop_zh_cn | desktop_en_us`
- minimal `stealth = true` bootstrap
- browser response status / headers
- 页面渲染后的 HTML 抓取

其中 browser `session` 当前会把同一个 session id 映射到稳定的 Playwright user data dir，
用于复用 cookies 和 local storage 这类浏览器态数据；相同 session id 的实际浏览器执行也会按 session 串行化，
避免共享 profile 目录时出现竞态。

当前 browser `Response` 会带上真实的导航 `status` 与响应头；`protocol` 继续表示
browser 执行路径，`ip_address` 与 `certificate` 由于 Playwright 当前接口限制仍保持为空。

当前已经支持的 browser 指纹能力边界：

- `fingerprint_profile` 当前只支持内置 profile：`desktop_zh_cn`、`desktop_en_us`
- `stealth = true` 当前会注入最小 bootstrap，覆盖 `navigator.webdriver`、`navigator.languages`、`navigator.platform`、最小 `window.chrome` 与 notifications permissions 查询补丁

当前仍未实现、并且会继续显式报错的能力：

- 自定义 `fingerprint_profile` 名称
- 更完整的第三方 stealth 套件或更高阶浏览器指纹伪装能力

如果当前构建没有启用 `browser` feature，browser request 会直接返回显式错误，不会再返回 stub response。

启用方式：

```toml
halo-spider = { version = "0.0.5", features = ["browser"] }
```

首次使用前需要安装 Playwright 浏览器：

```bash
npx playwright@1.58.2 install chromium firefox webkit
```

**已知限制：**
- HTML 解析暂不支持 XPath 选择器（当前 XPath 实现基于 XML 解析器，对不规范 HTML 容错性差）
- 建议在 HTML 场景下使用 CSS 选择器替代 XPath
- `ocr` 相关解析能力当前暂不实现
- DSL 配置面当前后置，优先补齐和稳定代码爬虫与共享底层能力

README 当前不再承诺字段级 DSL 配置说明。等配置面稳定后，再按模块补回。

## AI 选择器

使用 OpenAI API 进行智能内容提取：

```toml
[dependencies]
halo-spider = { version = "0.0.5", features = ["ai-selector"] }
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
        .with_timeout(jiff::SignedDuration::from_secs(30));
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
    .with_download_delay(jiff::SignedDuration::from_millis(200));  // 请求间延迟
```

参考 `examples/concurrency_control.rs` 或 `examples/README.md` 查看当前保留示例。

## 贡献指南

如果你想参与开发或了解项目的开发流程，请查看 [CONTRIBUTING.md](CONTRIBUTING.md)。
## License

MIT
