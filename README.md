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

- `Request`、`Response`、`download`、`scheduler`、`pipeline`、`store` 各自负责什么
- `scheduler::checkpoint::{Checkpoint, Counts, Persist}` 这组类型分别表示什么
- `scheduler` 当前为什么拆成“核心调度层”和“checkpoint 持久化层”
- 当前已经落地的底层能力、明确的边界以及暂缓项

## 底层能力概览

README 这里只保留总览；模块级细节统一放到 [docs/capabilities.md](/Users/xiaohan/soft/project/xiaohan/kun/docs/capabilities.md)。

当前已落地的底层能力：

- `Request` 已经是统一执行单元，覆盖 `method`、`headers`、`body`、`timeout`、`proxy`、`session`、request cookies 与 `follow` 继承语义
- `download::Http` 已接到真实的 timeout、proxy、redirect、cookie jar 与 session cookies 能力
- `download::Browser` 已具备最小可用浏览器下载能力，并支持统一 `Request` 上的 `method` / `body` / `headers` / `timeout` / `proxy` / cookies / `session`
- `Response.body` 与 `Response.text` 的语义已经明确并统一解码
- `scheduler::Memory` 与 `scheduler::Redis` 已把任务状态收口为 `ready / delayed / inflight`，并支持 `priority / depth` 排序；其中 `scheduler::Memory` 仍支持 `scheduler::checkpoint::Checkpoint` 导出/恢复
- 已提供 `scheduler::checkpoint::File`、`scheduler::checkpoint::Redis` 与 `scheduler::checkpoint::Memory`，用于文件、Redis 的 scheduler checkpoint 持久化；也已提供直接基于 Redis 的 durable scheduler
- `pipeline` 只负责 item 处理与过滤；最终持久化/投递走独立 `store` 边界，当前内置 `store::Memory`、`store::File`、`store::Sqlite`、`store::Webhook`、`store::Redis` 与 `store::Kafka`
- `Engine::new()` 默认使用 `store::File::default()`，结果会写到 `output/<spider_name>.jsonl`
- `Engine::default()` 等价于 `Engine::new()`
- `Engine::stats()` 已提供最小运行时计数快照：`request_count`、`response_count`、`error_count`、`retry_count`、`item_count`、`pipeline_drop_count`
- 已提供最小 `robots.txt` 策略：`Settings::with_robots_obey(true)` 开启后，会按 origin 缓存 `robots.txt`，并在下载前跳过不允许的请求
- plugin 自动装载当前只支持 `middleware` kind，其它 kind 先保留命名空间
- DSL 当前定位已经明确为“共享底层能力的配置化入口”，不是另一套独立运行时

当前仍待补齐的底层能力：

- 共享 validation 已支持字段路径解析与逐值校验（例如 `meta.title`、`authors[0].name`、`tags[]`、`articles[].title`），也已补显式文本/列表/对象约束（例如 `with_min_length(...)`、`with_min_items(...)`、`with_required_fields(...)`）、`ValidationTransform` 链式转换后再校验（例如 `trim`、`normalize_whitespace`、`parse_number`、`parse_bool`、`parse_datetime`）、对象子规则/列表成员子规则、`any_of / all_of / one_of / mutually_exclusive` 这类组合约束、`when_exists / when_missing / when_equals / when_not_equals` 这类条件约束，以及 `validate_fields_report()` 这种 collect-all 报告能力；validation 语义是显式启用的，只有传入的规则才会执行，字段缺失时也只有 `required` 或显式 `required_when_*` 条件命中时才报错，其它规则默认跳过；更高阶的运行时失败策略映射和更复杂的派生条件还没统一
- 当前已经有文件、Redis 两种 checkpoint 持久化，也已经有直接基于 Redis 的 durable scheduler；更强的分布式协调、事务语义与更高阶恢复策略还没统一
- 当前 item 链路已经明确为 `parse -> item -> pipeline -> store`；当前已经有文件、数据库、Webhook、Redis 与 Kafka 这些内置 `store` 实现，更丰富的文件格式与更高阶消息能力仍待继续补齐
- 当前 stats 还是内存内累计快照，尚未接 Prometheus/OpenTelemetry 这类 exporter；HTTP cache / conditional request 也还没补，并已明确放到 `P3`
- 当前 `robots.txt` 只补了最小策略：默认关闭、按 origin 内存缓存、支持 `User-agent` / `Allow` / `Disallow` 的前缀匹配；`Crawl-delay`、`Sitemap`、更完整通配符语义还没补
- HTML XPath、OCR、parse 后处理等 parser 能力仍未完全收敛；当前已补一组更完整的 query transform：`fallback(...)`、`fallback_many(...)`、`field(...)`、`index(...)`、`flatten()`、`compact()`、`trim()`、`first_non_empty()`、`skip(...)`、`take(...)`、`last()`、`dedup()`、`join(...)`、`split(...)`、`replace(...)`、`normalize_whitespace()`、`resolve_url(...)`、`parse_number()`、`parse_bool()`、`parse_json()`、`parse_datetime()`、`parse_datetime_with_format(...)`，以及最小 query 级断言：`require_non_empty()`、`require_one()`

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

    let mut engine = Engine::new().with_settings(settings);

    let handle = engine.shutdown_handle();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        handle.stop();
    });

    engine.run(&MySpider).await.unwrap();
}
```

## Scheduler 选择

如果默认组件足够，直接用 `Engine::new()`。
如果要自定义 `scheduler`、`http` 或 `browser`，再用 `Engine::from_parts(scheduler, http, browser)`。

- `Engine::new()` 默认就是 `scheduler::Memory + download::Http + download::Browser`
- `Engine::default()` 与 `Engine::new()` 等价，只是更偏 Rust trait 风格
- 如果你想保留默认 memory scheduler、但替换下载器，可以用 `Engine::with_downloaders(http, browser)`
- `checkpoint` 本身没有单独的 runtime 默认值；只有你显式启用 checkpoint 时，默认内置后端才是 `scheduler::checkpoint::File::default()`，路径是 `output/scheduler-checkpoint.json`
- 如果你想要“内存调度 + 文件 checkpoint”的便捷组合，可以直接用 `scheduler::checkpoint::Memory::default()`
- 如果你要从默认 checkpoint 文件恢复到 memory scheduler，使用 `scheduler::checkpoint::Memory::load_default().await?`
- 如果你要真正的 durable scheduler，可以直接传 `scheduler::Redis::new(...)`
- 如果你想自定义 checkpoint 后端，可以用 `scheduler::checkpoint::Memory::load(scheduler::checkpoint::Redis::new(...)).await?`
- 如果你想自定义 scheduler 或 checkpoint 后端，分别实现 `scheduler::Scheduler` 或 `scheduler::checkpoint::Persist` 即可
- 如果你更喜欢链式写法，可以从 `Engine::new()` 开始，再用 `.with_scheduler(...)`、`.with_checkpoint(...)` 或 `.load_checkpoint(...).await?`
- 完整 demo 见 `examples/custom_scheduler.rs`

```rust
use halo_spider::download::{Browser, Http};
use halo_spider::engine::Engine;
use halo_spider::scheduler;

// 1. 默认推荐：纯内存 scheduler
let engine = Engine::new();

// 2. 或者用 Rust 常见写法，语义和 Engine::new() 一样
let engine = Engine::default();

// 3. 默认 memory scheduler，但自定义 downloaders
let engine = Engine::with_downloaders(Http::default(), Browser::default());

// 4. memory scheduler + file checkpoint
let engine = Engine::new().with_checkpoint(scheduler::checkpoint::File::default());

// 5. 原生 durable Redis scheduler
let engine = Engine::new()
    .with_scheduler(scheduler::Redis::new("redis://127.0.0.1:6379", "kun:scheduler"));

// 6. 内存 scheduler + 自定义 Redis checkpoint
let scheduler = scheduler::checkpoint::Memory::load(
    scheduler::checkpoint::Redis::new(
        "redis://127.0.0.1:6379",
        "kun:scheduler:checkpoint",
    ),
)
.await?;
let engine = Engine::new().with_scheduler(scheduler);

// 7. 也可以先创建默认 engine，再链式替换 scheduler
let engine = Engine::new().with_scheduler(scheduler::Redis::new(
    "redis://127.0.0.1:6379",
    "kun:scheduler",
));

// 8. 如果要从已有 checkpoint 恢复，也可以直接链式加载
let engine = Engine::new()
    .load_checkpoint(scheduler::checkpoint::File::default())
    .await?;

// 9. 如果要自定义全部底层组件，用 from_parts(...)
let engine = Engine::from_parts(
    scheduler::Redis::new("redis://127.0.0.1:6379", "kun:scheduler"),
    Http::default(),
    Browser::default(),
);
```

## 示例

```bash
# 基础能力示例（统一使用 period.xml 场景）
cargo run --example period_xml_spider
cargo run --example memory
cargo run --example file
cargo run --example sqlite
cargo run --example webhook
cargo run --example redis
HALO_SPIDER_ES_URL=http://127.0.0.1:9200 cargo run --example elasticsearch
HALO_SPIDER_KAFKA_BROKERS=127.0.0.1:9092 cargo run --example kafka
cargo run --example custom_middleware
cargo run --example custom_scheduler
cargo run --example plugins_demo

# AI 选择器示例（需要 OPENAI_API_KEY 环境变量）
cargo run --example ai_extraction --features ai-selector

# 并发控制示例
cargo run --example concurrency_control
```

## Pipeline 与 Store

当前 item 执行链路固定为：

```text
parse -> item -> pipeline -> store
```

其中：

- `pipeline` 只负责 item 处理与过滤，例如 normalize、补默认值、丢弃无效 item
- `store` 负责最终持久化或投递，例如文件、数据库、HTTP API、消息队列

Engine 现在保留一个显式 `with_pipeline(...)` 插槽和一个显式 `with_store(...)` 插槽。
当前不再推荐 `with_pipeline((A, B))` 这类元组组合写法；如果确实需要多个 item
处理步骤，直接在你自己的 pipeline 类型里按顺序组合即可。

如果没有显式调用 `with_store(...)`，引擎默认使用
`store::File::default()`，并把结果写到 `output/<spider_name>.jsonl`。

需要跨请求透传上下文时，优先把数据放进 `request.meta`，并在最后一个
`parse()` / callback 里组装最终 item，而不是让 pipeline/store 充当隐藏状态通道。

如果你什么都不配，直接：

```rust
let engine = Engine::new();
```

那默认就是：

- `scheduler::Memory`
- `download::Http`
- `download::Browser`
- `store::File::default()`，输出到 `output/<spider_name>.jsonl`

`Store` 当前同时暴露 `write()` 和 `batch_write()` 两个入口：

- `write()` 负责单条 item 的最终写入或投递
- `batch_write()` 负责一批 item 的最终写入或投递
- engine 会把同一次 `parse()` / callback 输出里经过 pipeline 保留的 items 收成一批，并优先调用 `store.batch_write(...)`
- 默认 `Store::batch_write()` 会退回逐条调用 `write()`，所以最简单的 store 只实现单条写入也能正常工作
- 如果某个 store 底层支持原生批量写入，它可以覆盖 `batch_write()` 来减少锁竞争、文件打开次数或数据库往返次数

例如：

```rust
use halo_spider::store::{File, Sqlite};

let mut engine = Engine::from_parts(scheduler, http, browser)
    .with_store(Sqlite::new("output/items.db"));

let mut engine = Engine::from_parts(scheduler, http, browser)
    .with_store(File::new("output/items.jsonl"));
```

如果你需要 item 预处理，再额外调用 `with_pipeline(...)` 即可。

如果你要自定义自己的最终落地方式，直接实现 `store::Store` 即可。仓库里已经有一版完整示例：

- `examples/elasticsearch.rs`
  - 自定义 `ElasticsearchStore`
  - 单条走 `_doc`
  - 批量走 `_bulk`
  - 仍然挂在同一条 `parse -> item -> pipeline -> store` 主链上

当前内置 `store` 有：

- `store::Memory`
- `store::File`
- `store::Sqlite`
- `store::Webhook`
- `store::Redis`
- `store::Kafka`

当前 SQLite store 的最小语义是：

- `open()` 只负责建库建表，不会自动清空旧数据
- 每条 item 都会保留一份完整 `item_json`
- 显式映射的字段列按声明类型写入；缺失字段写 `NULL`
- 如果字段值类型和列类型不匹配，会返回显式 store error，而不是静默转换

当前 Webhook store 的最小语义是：

- 把完整 item JSON 通过 `POST` 或 `PUT` 推送到目标 HTTP endpoint
- 支持追加固定请求头
- 如果接口返回非 `2xx`，会返回显式 store error，而不是静默忽略失败

当前 Redis store 的最小语义是：

- 支持 `redis://` 连接 URL，并接住最小 `AUTH` / `SELECT` 语义
- `Redis::new(...)` 直接把完整 item JSON 用 `SADD` 写入目标 set
- `batch_write()` 会把一批 item JSON 合并成同一个 `SADD key value...` 命令
- 当前明确不做另一套消息输出 runtime；Redis 仍然只是同一条 `store` 边界上的一个内置实现

当前 Kafka store 的最小语义是：

- `Kafka::new(brokers, topic)` 把完整 item JSON 作为消息 value 发到指定 topic
- `batch_write()` 会在同一次 store 调用里连续发送多条 item JSON 消息
- 如果 Kafka producer 返回投递错误，store 返回显式 store error
- 当前不支持 message key、headers、显式 partition、事务、schema registry 或 consumer/group 这类更高阶 Kafka 语义

自定义 store 也走同一条主链。只要实现 `Store` trait，再通过
`Engine::with_store(...)` 挂进去即可。

例如，用户自己的 Elasticsearch / PostgreSQL store 都可以这样接入：

```rust
use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::store::Store;

#[derive(Clone)]
struct ElasticsearchStore {
    client: reqwest::Client,
    base_url: String,
    index: String,
}

impl ElasticsearchStore {
    fn new(base_url: impl Into<String>, index: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            index: index.into(),
        }
    }
}

impl Store for ElasticsearchStore {
    async fn write(&self, item: &Item, _spider_name: &str) -> Result<(), SpiderError> {
        self.client
            .post(format!("{}/{}/_doc", self.base_url.trim_end_matches('/'), self.index))
            .json(&item.to_json())
            .send()
            .await
            .map_err(|error| SpiderError::engine(format!("elasticsearch store request failed: {error}")))?;
        Ok(())
    }
}

let mut engine = Engine::from_parts(scheduler, http, browser)
    .with_store(ElasticsearchStore::new("http://127.0.0.1:9200", "period_items"));
```

如果你的自定义 store 底层本身支持批量写入，比如 Elasticsearch `_bulk`、ClickHouse
批量 insert、对象存储批量上传，也推荐一起覆盖 `batch_write()`。

后续更多文件格式与更完整消息语义也继续扩展在 `store` 这一层，而不是再拆新的输出运行时。

完整可运行示例见 `examples/memory.rs`、`examples/file.rs`、`examples/sqlite.rs`、`examples/webhook.rs`、`examples/redis.rs`、`examples/elasticsearch.rs` 与 `examples/kafka.rs`。

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
- `Response.body` 保留原始响应字节，`Response.text` 是从 `body` 派生出的解码文本；当前优先使用 BOM、`Content-Type charset` 与文档内编码声明，再回退到统计型 apparent encoding 猜测，最后才使用 UTF-8 lossy。
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

这里的 browser 定位仍然是“浏览器渲染型下载器”，不是通用自动化框架。
当前只保留导航、`wait_for`、统一 request 语义和最终 HTML 获取，不再继续暴露点击、滚动、脚本执行这类页面动作配置。

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
