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

按功能模块整理的能力说明见 [docs/capabilities.md](docs/capabilities.md)。

这份文档会集中解释：

- `Request`、`Response`、`download`、`scheduler`、`pipeline`、`store` 各自负责什么
- `scheduler::checkpoint::{Checkpoint, Counts, Persist}` 这组类型分别表示什么
- `scheduler` 当前为什么拆成“核心调度层”和“checkpoint 持久化层”
- 当前已经落地的底层能力、明确的边界以及暂缓项

## 底层能力概览

README 这里只保留总览；模块级细节统一放到 [docs/capabilities.md](docs/capabilities.md)。

当前已落地的底层能力：

- `Request` 已经是统一执行单元，覆盖 `method`、`headers`、`body`、`timeout`、`proxy`、`session`、request cookies、`callback`、`errback`、`kwargs` 与 `follow` 继承语义
- `download::Http` 已接到真实的 timeout、proxy、redirect、cookie jar 与 session cookies 能力
- `download::Browser` 已具备最小可用浏览器下载能力，并支持统一 `Request` 上的 `method` / `body` / `headers` / `timeout` / `proxy` / cookies / `session`
- `Response.body` 与 `Response.text` 的语义已经明确并统一解码
- Spider 现在除了 `start_urls()` / `build_start_urls()`，也可以直接覆写 `build_start_requests()` 返回完整 `Request`；默认仍然是把 URL 自动包成 `Request::new(...)`
- `scheduler::Memory` 与 `scheduler::Redis` 已把任务状态收口为 `ready / delayed / inflight`，并支持 `priority / depth` 排序；其中 `scheduler::Memory` 仍支持 `scheduler::checkpoint::Checkpoint` 导出/恢复，`scheduler::Redis` 也已补最小 `lease_timeout` stale inflight reclaim，把 `enqueue / claim / complete / requeue / reclaim` 这些关键状态迁移收口成 Redis 原子脚本，并提供 `snapshot()` 与按前缀批量读取 namespace 概览的运维入口
- 已提供 `scheduler::checkpoint::File`、`scheduler::checkpoint::Redis` 与 `scheduler::checkpoint::Memory`，用于文件、Redis 的 scheduler checkpoint 持久化；也已提供直接基于 Redis 的 durable scheduler
- `dedup` 已从默认 middleware 收口为显式 engine 组件；当前默认使用精确 `dedup::Memory`，也内置可选 `dedup::Bloom`，并可以通过 `Engine::with_dedup(...)` 切换为其它实现
- `robots` 已提升为显式 engine 组件；当前默认使用 `robots::Memory`，也可以通过 `Engine::with_robots(...)` 切换为其它实现
- `pipeline` 只负责 item 处理与过滤；最终持久化/投递走独立 `store` 边界，当前内置 `store::Memory`、`store::File`、`store::Sqlite`、`store::Webhook`、`store::Redis` 与 `store::Kafka`
- `Engine::new()` 默认使用 `store::File::default()`，结果会写到 `output/<spider_name>.jsonl`
- `Engine::default()` 等价于 `Engine::new()`
- `Engine::stats()` 已提供累计运行时计数快照：除了 `request_count`、`response_count`、`error_count`、`retry_count`、`item_count`、`pipeline_drop_count`，现在也包含 `dedup_reject_count`、`robots_disallow_count`、`robots_delay_count`、`http_cache_hit_count`、`http_cache_revalidate_count`、`http_cache_store_count`、`http_cache_miss_count` 与 `store_error_count`
- 已提供最小 `signals / extensions`：可以通过 `Engine::with_signal_listener(...)` 监听 `spider_opened`、`spider_closed`、`request_scheduled`、`response_received`、`item_scraped`、`spider_error` 这些 runtime 事件；如果只关心部分事件，也可以用 `Engine::with_signal_listener_for([...], ...)` 做 signal kind 过滤订阅；扩展侧同理可以用 `Engine::with_extension(...)` / `Engine::with_extension_for([...], ...)`，内置 `extensions::Summary`
- 已提供更完整一版 `robots.txt` 策略：`Settings::with_robots_obey(true)` 开启后，会按 origin 缓存 `robots.txt`，并在下载前处理 `Allow` / `Disallow`、`Crawl-delay`、`Request-rate`、更完整的 `User-agent group` 匹配，以及 `* / $` wildcard 规则；其中 `Request-rate` 当前按 `window / requests` 的均匀间隔最小 delay 解释，如果同时声明 `Crawl-delay` 与 `Request-rate`，则取更严格的 delay；`robots::Robot::sitemaps(...)` 也可读取声明的 sitemap URL；默认 cache backend 是 `robots::cache::Memory`，也可以通过 `robots::Memory::with_cache(...)` 替换为 `robots::cache::File` 或自定义实现；`robots::cache::File::default()` 的路径是 `output/robots-cache.json`；`robots::Memory` 当前默认按 `24h` 的 `cache_ttl` 复用 policy，过期后会尝试刷新，刷新失败时优先回退旧缓存；如果当前 origin 没有可用缓存且 `robots.txt` 临时不可用，默认按 `robots::UnavailablePolicy::AllowAll` 继续 fail-open，并对这类临时不可用结果按默认 `60s` 的 retry delay 做短暂退避，避免每个请求都重复抓取 `robots.txt`；调用方也可以显式切到 `DisallowAll`、覆盖 retry delay，或关闭这层退避；现在也可以通过 `robots::Memory::with_site_policy(robots::Site::..., ...)` 给指定站点 matcher 叠加站点策略，内置支持 `origin / host / pattern`，其中 `access` 与 `unavailable_policy` 由更具体 matcher 决定，同一 specificity 下后注册规则优先，`delay` 取更严格值，`sitemap` 做去重合并；如果再打开 `Settings::with_robots_sitemap_seeds(true)`，引擎启动时会把 robots 里声明的 sitemap / sitemapindex，包括常见的 `.xml.gz` 压缩 sitemap，一并解析成新的种子请求，并继承 start request 的共享请求能力；默认 `priority / depth` 仍是 `0 / 0`，但现在也可以通过 `with_robots_sitemap_seed_priority(...)` 和 `with_robots_sitemap_seed_depth(...)` 显式覆盖
- `robots` 这块如果想直接看 `Site::pattern / host / origin` 的接法，可以运行 `examples/robots_site_policy.rs`
- 已提供最小 `AutoThrottle`：`Settings::with_auto_throttle(true)` 开启后，会按 origin 基于延迟、错误和目标并发动态调整下一次下载间隔；此时 `download_delay` 表示初始/最小 delay，`with_auto_throttle_max_delay(...)` 表示最大 delay
- 已提供一版更完整的 `HTTP cache / conditional request`：默认还是内存 backend，也已支持内置 `middleware::http_cache::File` 持久化 backend；`Settings::with_http_cache(true)` 开启后，同一 HTTP `GET` 请求会基于已缓存的 `ETag / Last-Modified` 自动补 `If-None-Match / If-Modified-Since`；命中 `304 Not Modified` 时，在 `response` 策略下会回填缓存 body，并给 `Response.flags` 增加 `http_cache`；当前也支持 `ttl` 和 `validators / response` 两种缓存策略
- plugin 自动装载当前只支持 `middleware` kind；当前已知但暂未自动装载的 kind 统一收口为 `store`、`scheduler`、`dedup`、`robots`、`http`、`browser`
- DSL 当前定位已经明确为“共享底层能力的配置化入口”，不是另一套独立运行时

当前仍待补齐的底层能力：

- 观测能力还不完整：虽然已经有细粒度 `stats`、`signals / extensions` 和 `Engine::with_stats_reporter(...)`，但还没有内置 Prometheus / OpenTelemetry exporter、trace 链路、持久化事件总线或跨 job 运维视角。
- durable scheduler 已经可用，但更强的分布式协调、事务边界、跨 worker ownership / heartbeat 运维语义还没完全统一收口。
- `store` 边界已经建立，但更丰富的文件格式、更高阶消息语义和更多内置外部系统适配还没有继续铺开。
- browser 已经支持内置 profile、结构化自定义 profile 与显式 `session_reuse`；当前剩余缺口主要是更高阶第三方 stealth 套件与跨 engine 更完整的品牌级指纹伪装。
- validation 本身已经比较完整，但“校验失败如何映射到 runtime 行为”这层统一策略还没完全收口。
- plugin 自动装载当前仍只支持 `middleware` kind；`store`、`scheduler`、`dedup`、`robots`、`http`、`browser` 这些 kind 还没有真正自动接线。
- DSL 继续后置，尚未完全追平代码爬虫已经具备的共享 request / parse / schedule / validation 能力。

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
如果想保留默认 scheduler、只替换下载器，优先用 `.with_http(...)` / `.with_browser(...)`。
如果要连 `scheduler` 一起自定义，再用 `Engine::from_parts(scheduler, http, browser)`。
如果要替换默认去重实现，再继续链 `.with_dedup(...)`。
如果要替换默认 robots policy，再继续链 `.with_robots(...)`。
如果要监听全部 runtime 事件，再继续链 `.with_signal_listener(...)`。
如果只想监听部分 signal kind，再用 `.with_signal_listener_for([...], ...)`。
如果要挂扩展，再继续链 `.with_extension(...)` 或 `.with_extension_for([...], ...)`。

- `Engine::new()` 默认就是 `scheduler::Memory + download::Http + download::Browser + dedup::Memory + robots::Memory`
- `Engine::default()` 与 `Engine::new()` 等价，只是更偏 Rust trait 风格
- 如果你想只替换 HTTP 下载器，可以用 `.with_http(...)`
- 如果你想只替换 browser 下载器，可以用 `.with_browser(...)`
- 如果你想同时替换两个下载器，可以链 `.with_http(...).with_browser(...)`
- `Engine::with_downloaders(http, browser)` 继续保留，作为“默认 memory scheduler + 一次替换两个下载器”的快捷写法
- 当前不再单独引入 `with_queue(...)`；任务排队、ready/delayed/inflight 流转与恢复边界，统一都收口在 `scheduler` 组件里
- 如果你想关闭默认去重，可以显式用 `.with_dedup(dedup::Noop)`
- 如果你想用有界内存的近似去重，可以显式用 `.with_dedup(dedup::Bloom::default())`
- 如果你想保留框架接线、但替换 dedup 算法，可以显式用 `.with_dedup(...)`
- 如果你是手动往引擎里塞 request，优先用 `engine.enqueue(request).await?`；直接调 `engine.scheduler.enqueue(...)` 属于低层 escape hatch，会绕过 dedup 组件
- `robots` 是否启用和使用哪个 user-agent 仍由 `Settings::with_robots_obey(...)` / `Settings::with_robots_user_agent(...)` 控制；`.with_robots(...)` 负责替换具体 robots policy 实现
- 如果你想拿到最原始的 runtime 事件流，可以显式用 `.with_signal_listener(...)`
- 如果你只关心部分 signal kind，可以用 `.with_signal_listener_for([...], ...)`
- 如果你想挂语义更清楚的运行时扩展，可以显式用 `.with_extension(...)` 或 `.with_extension_for([...], ...)`；它底层复用同一条 signal bus
- `checkpoint` 本身没有单独的 runtime 默认值；只有你显式启用 checkpoint 时，默认内置后端才是 `scheduler::checkpoint::File::default()`，路径是 `output/scheduler-checkpoint.json`
- 如果你想要“内存调度 + 文件 checkpoint”的便捷组合，可以直接用 `scheduler::checkpoint::Memory::default()`
- 如果你要从默认 checkpoint 文件恢复到 memory scheduler，使用 `scheduler::checkpoint::Memory::load_default().await?`
- 如果你要真正的 durable scheduler，可以直接传 `scheduler::Redis::new(...)`
- `scheduler::Redis` 默认会给 `inflight` task 一个最小 `lease_timeout`，worker 崩溃或长时间失联后，后续访问同 namespace 时会把 stale `inflight` task 回收到 `ready / delayed`
- `scheduler::Redis` 现在会通过 Redis 脚本原子完成 `claim / complete / requeue / reclaim` 这类关键迁移；多个 worker 共享同一个 namespace 时，不会再因为本地“先读 ready 再分步搬运”而重复领取同一条 task
- `scheduler::Redis` 现在还显式支持 `worker_id`、runtime lease ownership 校验，以及 engine 运行中的 heartbeat 续租
- `scheduler::Redis::snapshot().await?` 读取的是某一个 namespace 当前这一刻的 durable scheduler 即时状态；它和 `Engine::stats()` 不一样，后者仍然是单个 engine 实例生命周期内的累计计数
- `snapshot.inflight_tasks` 会直接带出每条 inflight task 的 `task_id / url / worker_id / lease_id / deadline / priority / depth / ready_at`，运维时不需要再手工回读底层 Redis key
- 如果同一个 Redis 里同时跑多个 job，可以用 `scheduler::Redis::namespaces_with_prefix(...)` 先按前缀发现 namespace，再用 `scheduler::Redis::namespace_snapshots_with_prefix(...)` 批量读取各 job 的运行时概览
- 如果你想调整这层恢复窗口，可以用 `.with_lease_timeout(...)`；如果你想显式指定 worker 身份或 heartbeat 节奏，可以再配 `.with_worker_id(...)`、`.with_heartbeat_interval(...)`；如果你明确不想要这层自动回收，也可以用 `.without_lease_timeout()`
- 如果你想自定义 checkpoint 后端，可以用 `scheduler::checkpoint::Memory::load(scheduler::checkpoint::Redis::new(...)).await?`
- `checkpoint` 仍然只是静态快照恢复边界；它不会替代 durable scheduler 的 runtime reclaim
- 如果你想自定义 scheduler 或 checkpoint 后端，分别实现 `scheduler::Scheduler` 或 `scheduler::checkpoint::Persist` 即可
- 如果你更喜欢链式写法，可以从 `Engine::new()` 开始，再用 `.with_scheduler(...)`、`.with_checkpoint(...)` 或 `.load_checkpoint(...).await?`
- 完整 demo 见 `examples/custom_scheduler.rs`
- 分布式运行说明见 `docs/distributed_scheduler.md`

```rust
use halo_spider::dedup;
use halo_spider::download::{Browser, Http};
use halo_spider::engine::Engine;
use halo_spider::robots;
use halo_spider::scheduler;
use halo_spider::settings::Settings;

// 1. 默认推荐：纯内存 scheduler
let engine = Engine::new();

// 2. 或者用 Rust 常见写法，语义和 Engine::new() 一样
let engine = Engine::default();

// 3. 默认 memory scheduler，但自定义 downloaders
let engine = Engine::with_downloaders(Http::default(), Browser::default());

// 4. 也可以按组件分别替换下载器
let engine = Engine::new()
    .with_http(Http::default())
    .with_browser(Browser::default());

// 5. 默认 dedup::Memory，但可以显式替换 dedup 组件
let engine = Engine::new().with_dedup(
    dedup::Memory::new().with_keys([dedup::Key::Url, dedup::Key::Method]),
);

// 6. 如果想禁用 dedup，也可以显式换成 Noop
let engine = Engine::new().with_dedup(dedup::Noop);

// 7. 如果想改成 Bloom dedup，也可以显式替换
let engine = Engine::new().with_dedup(
    dedup::Bloom::new()
        .with_expected_items(500_000)
        .with_false_positive_rate(0.01),
);

// 8. memory scheduler + file checkpoint
let engine = Engine::new().with_checkpoint(scheduler::checkpoint::File::default());

// 9. 替换默认 robots policy；真正执行仍要开启 robots_obey
let engine = Engine::new()
    .with_robots(robots::Noop)
    .with_settings(Settings::default().with_robots_obey(true));

// 10. 原生 durable Redis scheduler
let engine = Engine::new()
    .with_scheduler(
        scheduler::Redis::new("redis://127.0.0.1:6379", "kun:scheduler")
            .with_worker_id("news-worker-a")
            .with_lease_timeout(jiff::SignedDuration::from_secs(30))
            .with_heartbeat_interval(jiff::SignedDuration::from_secs(10)),
    );

// 11. 内存 scheduler + 自定义 Redis checkpoint
let scheduler = scheduler::checkpoint::Memory::load(
    scheduler::checkpoint::Redis::new(
        "redis://127.0.0.1:6379",
        "kun:scheduler:checkpoint",
    ),
)
.await?;
let engine = Engine::new().with_scheduler(scheduler);

// 12. 也可以先创建默认 engine，再链式替换 scheduler
let engine = Engine::new().with_scheduler(scheduler::Redis::new(
    "redis://127.0.0.1:6379",
    "kun:scheduler",
));

// 13. 如果要从已有 checkpoint 恢复，也可以直接链式加载
let engine = Engine::new()
    .load_checkpoint(scheduler::checkpoint::File::default())
    .await?;

// 14. 如果要挂最小 runtime extension，可以直接复用内置 Summary
let engine = Engine::new().with_extension(halo_spider::extensions::Summary);

// 15. 如果只关心 spider_closed，再做过滤订阅
let engine = Engine::new().with_signal_listener_for(
    [halo_spider::signals::Kind::SpiderClosed],
    my_listener,
);

// 16. 如果要自定义全部底层组件，用 from_parts(...)
let engine = Engine::from_parts(
    scheduler::Redis::new("redis://127.0.0.1:6379", "kun:scheduler"),
    Http::default(),
    Browser::default(),
)
.with_dedup(dedup::Noop);
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
cargo run --example robots_site_policy
HALO_SPIDER_ES_URL=http://127.0.0.1:9200 cargo run --example elasticsearch
HALO_SPIDER_KAFKA_BROKERS=127.0.0.1:9092 cargo run --example kafka
cargo run --example custom_dedup
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

Engine 现在保留显式 `with_dedup(...)`、`with_robots(...)`、
`with_pipeline(...)` 与 `with_store(...)` 这些组件插槽。
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
- `dedup::Memory`
- `robots::Memory`
- `store::File::default()`，输出到 `output/<spider_name>.jsonl`

关于 dedup 默认值，这里也明确一下：

- `Engine::new()` 继续默认用精确 `dedup::Memory`
- `dedup::Bloom` 是显式 opt-in，不默认替换
- 原因是默认行为优先保 correctness，不默认引入布隆误判导致的潜在漏抓

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

当前 `store::File` 的最小增强语义是：

- 默认仍然写紧凑的 JSON Lines
- 可以通过 `with_format(store::FileFormat::PrettyJsonBlocks)` 切到更适合人工查看的 pretty block 形式
- 可以通过 `with_rotate_items(...)` 或 `with_rotate_bytes(...)` 把输出切分成编号文件，例如 `items-0001.jsonl`
- 这些增强仍然只发生在同一个 `store::File` 边界上，不引入第二套文件输出 runtime

当前 SQLite store 的最小语义是：

- `open()` 只负责建库建表，不会自动清空旧数据
- 每条 item 都会保留一份完整 `item_json`
- 显式映射的字段列按声明类型写入；缺失字段写 `NULL`
- 如果字段值类型和列类型不匹配，会返回显式 store error，而不是静默转换

当前 Webhook store 的最小语义是：

- 把完整 item JSON 通过 `POST` 或 `PUT` 推送到目标 HTTP endpoint
- 支持追加固定请求头
- 支持 `with_retry_limit(...)` 与 `with_retry_backoff(...)`
- 当前只对请求错误和 `429 / 5xx` 做重试；其它非 `2xx` 仍然直接报错
- 如果接口返回非 `2xx`，会返回显式 store error，而不是静默忽略失败

当前 Redis store 的最小语义是：

- 支持 `redis://` 连接 URL，并接住最小 `AUTH` / `SELECT` 语义
- `Redis::new(...)` 直接把完整 item JSON 用 `SADD` 写入目标 set
- `batch_write()` 会把一批 item JSON 合并成同一个 `SADD key value...` 命令
- 当前明确不做另一套消息输出 runtime；Redis 仍然只是同一条 `store` 边界上的一个内置实现

当前 Kafka store 的最小语义是：

- `Kafka::new(brokers, topic)` 把完整 item JSON 作为消息 value 发到指定 topic
- 支持 `with_key(...)` / `with_key_field(...)`
- 支持 `with_header(...)` / `with_header_field(...)`
- `batch_write()` 会在同一次 store 调用里连续发送多条 item JSON 消息
- 如果 Kafka producer 返回投递错误，store 返回显式 store error
- 当前仍不支持显式 partition、事务、schema registry 或 consumer/group 这类更高阶 Kafka 语义

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

当前内置维护范围也明确一下：

- 框架内置继续维护 `Memory / File / Sqlite / Webhook / Redis / Kafka`
- 更专门的数据库、对象存储、第三方 API、复杂 MQ 语义，优先继续通过用户自定义 `Store` 扩展

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
- `step.fetch.request` / `step.fetch.browser` 现在也是往同一个 `Request` 模型上做显式覆盖：无论是 rules 起始请求，还是 DSL step 继续产出的 follow request，都会先走共享 `Request` / `follow()` 语义，再应用目标 step 的 request 配置。
- `step.validate` 走共享 validation；DSL step 产出的 item 也还是继续走统一的 `pipeline -> store` 链路。
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
- built-in `fingerprint_profile = desktop_zh_cn | desktop_en_us | desktop_en_gb | desktop_ja_jp | desktop_de_de | desktop_fr_fr`
- structured `custom_fingerprint_profile`
- explicit `session_reuse = storage | context | page`
- richer `stealth = true` bootstrap
- browser response status / headers
- 页面渲染后的 HTML 抓取

其中 browser `session` 当前会把同一个 session id 映射到稳定的 Playwright user data dir，
用于复用 cookies 和 local storage 这类浏览器态数据；同时 `session_reuse` 现在也可以显式选择：

- `storage`：只复用稳定 user data dir，每次请求仍新建并关闭 context/page
- `context`：同一 session 复用 live context，但每次请求新建 page
- `page`：同一 session 复用 live context 和同一张 live page

user data dir、临时 profile 目录和会话锁这条实现路径也已经收口到更适合 async runtime 的处理方式；相同 session id 的实际浏览器执行仍会按 session 串行化，避免共享 profile 目录或 live runtime 时出现竞态。

当前 browser `Response` 会带上真实的导航 `status` 与响应头；`protocol` 继续表示
browser 执行路径，`ip_address` 与 `certificate` 由于 Playwright 当前接口限制仍保持为空。

这里的 browser 定位仍然是“浏览器渲染型下载器”，不是通用自动化框架。
当前只保留导航、`wait_for`、统一 request 语义和最终 HTML 获取，不再继续暴露点击、滚动、脚本执行这类页面动作配置。

当前已经支持的 browser 指纹能力边界：

- `fingerprint_profile` 当前只支持内置 profile：`desktop_zh_cn`、`desktop_en_us`、`desktop_en_gb`、`desktop_ja_jp`、`desktop_de_de`、`desktop_fr_fr`
- `custom_fingerprint_profile` 可以直接传结构化 profile，不必先注册新的内置 preset 名称
- 这些 profile 会稳定映射 `user_agent`、`locale`、`timezone`、`accept-language`、`languages`、`platform`
- `stealth = true` 当前会注入一版更完整但仍然克制的 bootstrap，覆盖 `navigator.webdriver`、`navigator.language(s)`、`navigator.platform`、`navigator.vendor`、`hardwareConcurrency`、`deviceMemory`、`maxTouchPoints`、`plugins`、`mimeTypes`、`pdfViewerEnabled`、screen depth、notifications permissions 查询补丁，以及 Chromium 路线上的最小 `window.chrome` / `navigator.userAgentData`
- 这组 profile 和 stealth 仍然只是稳定内置 preset，不追求跨所有 Playwright engine 的“完全品牌一致”高阶伪装能力

当前仍未实现、并且会继续显式报错的能力：

- 自定义 `fingerprint_profile` 名称注册机制
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

最小使用示例：

```rust
use halo_spider::request::{browser, Request};
use jiff::SignedDuration;

let request = Request::browser("https://example.com/app")
    .with_timeout(SignedDuration::from_secs(15))
    .with_session("news-browser")
    .with_browser(
        browser::Config::default()
            .with_engine(browser::Engine::Chromium)
            .with_wait_for("#app")
            .with_custom_fingerprint_profile(
                browser::FingerprintProfile::new()
                    .with_locale("ja-JP")
                    .with_timezone("Asia/Tokyo")
                    .with_accept_language("ja-JP,ja;q=0.9")
                    .with_languages(["ja-JP", "ja", "en-US", "en"]),
            )
            .with_session_reuse(browser::SessionReuse::Context)
            .with_stealth(true),
    );
```

**当前边界：**
- `HTML` 与 `XML` 现在都支持 `XPath`；HTML 响应会先被解析并规范化成稳定 DOM，再执行 `one()`、`all()`、`text()`、`html()` 与 `attr()` 这组统一提取语义
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

如果你想改成按站点反馈动态调速，可以直接开启最小 `AutoThrottle`：

```rust
let settings = Settings::default()
    .with_auto_throttle(true)
    .with_download_delay(jiff::SignedDuration::from_millis(200))   // 初始/最小 delay
    .with_auto_throttle_target_concurrency(1.0)
    .with_auto_throttle_max_delay(jiff::SignedDuration::from_secs(5));
```

开启后，引擎会把固定 `interval_gate` 收口成 `auto_throttle` 中间件：同 origin 的慢响应、`429 / 5xx` 和下载异常都会抬高后续 delay，恢复正常后再逐步回落。

如果你想让同一个 HTTP `GET` 请求自动走最小条件请求缓存，也可以直接开启：

```rust
use halo_spider::engine::Engine;
use halo_spider::settings::Settings;

let engine = Engine::new().with_settings(
    Settings::default()
        .with_http_cache(true)
        .with_http_cache_ttl(jiff::SignedDuration::from_hours(12))
        .with_http_cache_strategy(halo_spider::middleware::http_cache::Strategy::Response),
);
```

当前这层 `http_cache` 的边界也比较明确：

- 只作用于 HTTP `GET` 请求
- 当前 key 语义是规范化后的完整 URL，包含 `request.http.query`
- 默认 backend 是进程内 `middleware::http_cache::Memory`，也可以通过 `Settings::with_http_cache_file(...)` 或 `HttpCache::with_cache(...)` 换成文件或自定义 backend
- 默认按 `24h` 的 `ttl` 复用缓存条目；也可以通过 `with_http_cache_ttl(...)` 覆盖，或 `without_http_cache_ttl()` 关闭自动过期
- 当前支持 `validators` 和 `response` 两种策略：前者只缓存 `ETag / Last-Modified`，后者还会缓存响应 body，并在 `304 Not Modified` 时回填成正常 `Response`
- 当前仍然不做 `Cache-Control` / `Expires` / `Vary` 这类更完整 HTTP 缓存语义

完整可运行示例见 `examples/http_cache.rs`，里面把 `ttl`、`strategy`、`file backend` 和最终 `Engine::stats()` 一起串起来了。
如果你想自己实现 cache backend，则看 `examples/custom_http_cache.rs`，它演示了如何实现 `middleware::http_cache::Cache`，再通过 `HttpCache::with_cache(...)` 接到引擎里。

参考 `examples/concurrency_control.rs` 或 `examples/README.md` 查看当前保留示例。

## 贡献指南

如果你想参与开发或了解项目的开发流程，请查看 [CONTRIBUTING.md](CONTRIBUTING.md)。
## License

MIT
