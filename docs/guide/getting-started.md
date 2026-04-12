# 入门

[返回使用手册](../guide.md)

## 当前状态

- 库代码位于 `src/`
- 示例位于 `examples/`
- 项目首页位于 [README.md](../../README.md)
- 功能说明文档位于 [capabilities.md](../capabilities.md)
- 运维 / 观测指南位于 [operations.md](../operations.md)
- 当前规范源位于 `openspec/specs/`
- 后续需求、方案、任务统一从 `openspec/changes/` 发起
- `openspec init` 生成的协作入口位于 `.claude/commands/opsx/` 与 `.codex/skills/`

## 功能说明

按功能模块整理的能力说明见 [capabilities.md](../capabilities.md)。
如果你更关心“数据怎么拿、跨 job 怎么看、pause/release/purge 怎么做”，直接看 [operations.md](../operations.md)。

这章主要先建立整体心智模型：

- `Request`、`Response`、`download`、`scheduler`、`pipeline`、`store` 各自负责什么
- `scheduler::checkpoint::{Checkpoint, Counts, Persist}` 这组类型分别表示什么
- `scheduler` 当前为什么拆成“核心调度层”和“checkpoint 持久化层”
- 当前已经落地的底层能力、明确的边界以及暂缓项

## 底层能力概览

这章负责给出整体用法和边界概览；模块级细节继续放到 [capabilities.md](../capabilities.md)。

当前已落地的底层能力：

- `Request` 已经是统一执行单元，覆盖 `method`、`headers`、`body`、`timeout`、`proxy`、`session`、request cookies、`callback`、`errback`、`cb_kwargs` 与 `follow` 继承语义
- `download::Http` 已接到真实的 timeout、proxy、redirect、cookie jar 与 session cookies 能力
- `download::Browser` 已具备最小可用浏览器下载能力，并支持统一 `Request` 上的 `method` / `body` / `headers` / `timeout` / `proxy` / cookies / `session`
- `Response.body` 与 `Response.text` 的语义已经明确并统一解码；同时也提供了 `meta` / `cb_kwargs` shortcut，以及 `urljoin` / `follow` / `follow_all` 这组子请求 helper
- Spider 现在除了 `start_urls()` / `build_start_urls()`，也可以直接覆写 `build_start_requests()` 返回完整 `Request`；默认仍然是把 URL 自动包成 `Request::new(...)`
- `scheduler::Memory`、`scheduler::Sqlite` 与 `scheduler::Redis` 已把任务状态收口为 `ready / delayed / inflight`，并支持 `priority / depth` 排序；其中 `scheduler::Memory` 仍支持 `scheduler::checkpoint::Checkpoint` 导出/恢复，`scheduler::Sqlite` 提供单机 durable scheduler，`scheduler::Redis` 提供共享 durable scheduler；现在所有 scheduler 后端统一通过 `Scheduler` 暴露 `checkpoint() / counts() / snapshot() / scopes() / snapshots() / overview()` 这组读能力，并通过 `scheduler::Control` 暴露 `pause_scope() / resume_scope() / release_scope() / purge_scope()` 这组运维控制入口；`Sqlite / Redis` 这类可见多 scope 的 durable backend 可以返回/控制多个 scope，本地 `Memory` 则返回/控制当前 scope；如果需要更多后端扩展支持，可以参考 `examples/custom_scheduler_mysql.rs` 自定义实现
- `scheduler::checkpoint::File`、`scheduler::checkpoint::Redis` 与 `scheduler::checkpoint::Memory` 用于文件、Redis 的 scheduler checkpoint 持久化；也支持直接基于 Redis 的 durable scheduler
- `dedup` 当前收口为 enqueue admission 阶段的默认中间件后端；`Engine::new()` 默认挂精确 `dedup::Memory`，也内置可选 `dedup::Bloom`，并可以通过 `Engine::with_dedup(...)` 替换这条默认去重后端
- `robots` 已提升为显式 engine 组件；当前默认使用 `robots::Memory`，也可以通过 `Engine::with_robots(...)` 切换为其它实现
- `pipeline` 只负责 item 处理与过滤；最终持久化/投递走独立 `store` 边界，当前内置 `store::Memory`、`store::File`、`store::Sqlite`、`store::Webhook`、`store::Redis` 与 `store::Kafka`
- `Engine::new()` 默认使用 `store::File::default()`，结果会写到 `output/<spider_name>.jsonl`
- `Engine::default()` 等价于 `Engine::new()`
- `Engine::stats()` 返回累计运行时计数快照：除了 `request_count`、`response_count`、`error_count`、`retry_count`、`item_count`、`pipeline_drop_count`，现在也包含 `dedup_reject_count`、`robots_disallow_count`、`robots_delay_count`、`http_cache_hit_count`、`http_cache_revalidate_count`、`http_cache_store_count`、`http_cache_miss_count`、`store_error_count`，以及 `scheduler_claim_count / scheduler_complete_count / scheduler_requeue_count / scheduler_heartbeat_count / scheduler_lease_lost_count`
- `signals / extensions` 已接到引擎运行时：可以通过 `Engine::with_signal_listener(...)` 监听 `spider_opened`、`spider_closed`、`request_scheduled`、`response_received`、`item_scraped`、`spider_error`，以及统一的 `scheduler_event` runtime 事件；如果只关心部分事件，也可以用 `Engine::with_signal_listener_for([...], ...)` 做 signal kind 过滤订阅；扩展侧同理可以用 `Engine::with_extension(...)` / `Engine::with_extension_for([...], ...)`，内置 `extensions::Summary`
- `telemetry` 是统一导出边界：`Engine::with_telemetry(...)` 会同时接入 engine stats 与 scheduler runtime 事件；内置 `telemetry::Collector`、`telemetry::File`、`telemetry::Prometheus`、`telemetry::OpenTelemetry` 和 `telemetry::Fanout`
- `robots.txt` 策略支持 `Allow` / `Disallow`、`Crawl-delay`、`Request-rate`、`User-agent group` 匹配，以及 `* / $` wildcard 规则；`Request-rate` 当前按 `window / requests` 的均匀间隔最小 delay 解释，如果同时声明 `Crawl-delay` 与 `Request-rate`，则取更严格的 delay；`robots::Robot::sitemaps(...)` 也可读取声明的 sitemap URL；默认 cache backend 是 `robots::cache::Memory`，也可以通过 `robots::Memory::with_cache(...)` 替换为 `robots::cache::File` 或自定义实现；`robots::cache::File::default()` 的路径是 `output/robots-cache.json`；`robots::Memory` 默认按 `24h` 的 `cache_ttl` 复用 policy，过期后会尝试刷新，刷新失败时优先回退旧缓存；如果当前 origin 没有可用缓存且 `robots.txt` 临时不可用，默认按 `robots::UnavailablePolicy::AllowAll` 继续 fail-open，并对这类临时不可用结果按默认 `60s` 的 retry delay 做短暂退避，避免每个请求都重复抓取 `robots.txt`；调用方也可以显式切到 `DisallowAll`、覆盖 retry delay，或关闭这层退避；现在也可以通过 `robots::Memory::with_site_policy(robots::Site::..., ...)` 给指定站点 matcher 叠加站点策略，内置支持 `origin / host / pattern`，其中 `access` 与 `unavailable_policy` 由更具体 matcher 决定，同一 specificity 下后注册规则优先，`delay` 取更严格值，`sitemap` 做去重合并；如果再打开 `Config::with_robots_sitemap_seeds(true)`，引擎启动时会把 robots 里声明的 sitemap / sitemapindex，包括常见的 `.xml.gz` 压缩 sitemap，一并解析成新的种子请求，并继承 start request 的共享请求能力；默认 `priority / depth` 仍是 `0 / 0`，但现在也可以通过 `with_robots_sitemap_seed_priority(...)` 和 `with_robots_sitemap_seed_depth(...)` 显式覆盖
- `robots` 这块如果想直接看 `Site::pattern / host / origin` 的接法，可以运行 `examples/robots_site_policy.rs`
- `AutoThrottle` 支持按 origin 基于延迟、错误和目标并发动态调整下一次下载间隔；`Config::with_auto_throttle(true)` 开启后，`download_delay` 表示初始/最小 delay，`with_auto_throttle_max_delay(...)` 表示最大 delay
- `HTTP cache / conditional request` 支持内存 backend 和内置 `middleware::http_cache::File` 持久化 backend；`Config::with_http_cache(true)` 开启后，同一 HTTP `GET` 请求会基于已缓存的 `ETag / Last-Modified` 自动补 `If-None-Match / If-Modified-Since`；命中 `304 Not Modified` 时，在 `response` 策略下会回填缓存 body，并给 `Response.flags` 增加 `http_cache`；当前也支持 `ttl` 和 `validators / response` 两种缓存策略
- plugin 当前定位为 `middleware` 的声明式装配：manifest / registry / factory / `load_plugins()` 这条链路已可用；核心组件继续走 trait + engine 显式注入，不把 plugin 当通用组件扩展机制
- DSL 当前定位已经明确为“共享底层能力的配置化入口”，不是另一套独立运行时；在新 v1 设计里，这类共享能力会统一映射到具体的 `engine.*` middleware 配置

当前共享底层能力：

- rules DSL v1 已经共享底层 `Request / scheduler / validation / store` 主链；其中 `output.validate.required` / `fields` 已接入 step validator。

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
use halo_spider::settings::Config;
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
    halo_spider::trace::init_console();

    let config = Config::default()
        .with_download_delay(SignedDuration::from_millis(200))
        .with_idle_timeout(SignedDuration::from_secs(5));

    let mut engine = Engine::new().with_config(config);

    let handle = engine.shutdown_handle();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        handle.stop();
    });

    engine.run(&MySpider).await.unwrap();
}
```

## Request Middleware 最小示例

如果你想给某一类请求单独挂重试、下载前控制，或者给自定义 middleware 指定顺序，推荐直接按具体 middleware 写，不再用组级入口。

```rust
use halo_spider::engine::Engine;
use halo_spider::middleware::Stage;
use halo_spider::request::Request;
use halo_spider::settings::Config;
use halo_spider::value::Value;
use std::collections::BTreeMap;

pub const CUSTOM_HEADER: &str = "custom_header";

let engine = Engine::new()
    .register_middleware(CUSTOM_HEADER, |options| {
        Ok(Box::new(CustomHeaderMiddleware::new(options)))
    })
    .with_config(
        Config::default().with_request_middleware(
            CUSTOM_HEADER,
            halo_spider::middleware::Config {
                enabled: true,
                stage: Stage::Download,
                order: 115,
                options: BTreeMap::new(),
            },
        ),
    );

let retry_cfg = BTreeMap::from([
    ("count".to_string(), Value::Number(2.0)),
    (
        "status".to_string(),
        Value::Array(vec![Value::Number(429.0), Value::Number(500.0)]),
    ),
]);

let interval_cfg = BTreeMap::from([("interval".to_string(), Value::Number(300.0))]);

let header_cfg = BTreeMap::from([(
    "headers".to_string(),
    Value::Object(BTreeMap::from([(
        "X-Channel".to_string(),
        Value::String("news".to_string()),
    )])),
)]);

let request = Request::new("https://example.com/detail")
    .with_retry_by_status(retry_cfg, 200)
    .with_interval(interval_cfg, 120)
    .with_middleware_options_ordered(CUSTOM_HEADER, header_cfg, 118);
```

这段示例里有两个关键点：

- `Config.order = 115`
  - 表示 `custom_header` 这条 middleware 的默认顺序是 `115`
- `with_middleware_options_ordered(CUSTOM_HEADER, ..., 118)`
  - 表示这一次 request 临时把它改成 `118`

可以把这套规则记成一句话：

- `Config.order` 是默认顺序
- `Request::with_xxx(..., order)` 是单次覆盖顺序

如果某条 request 不想让某个 middleware 生效，可以直接：

```rust
let request = Request::new("https://example.com/detail")
    .skip([CUSTOM_HEADER]);
```

内置 middleware 也是同一套写法，例如：

```rust
use halo_spider::middleware::{DEDUP, RETRY_BY_STATUS};

let request = Request::new("https://example.com/detail")
    .skip([DEDUP, RETRY_BY_STATUS]);
```

如果你写的是 rules DSL，对应就是：

```yaml
request:
  url: "https://example.com/detail"
  skip:
    - "dedup"
    - "retry_by_status"
```
