# Scheduler 与 Runtime

[返回使用手册](../guide.md)

## Scheduler 选择

如果默认组件足够，直接用 `Engine::new()`。
如果想保留默认 scheduler、只替换下载器，优先用 `.with_http(...)` / `.with_browser(...)`。
如果要连 `scheduler` 一起自定义，再用 `Engine::from_parts(scheduler, http, browser)`。
如果要替换默认输出落点，再继续链 `.with_store(...)`。
如果要注册多个可路由输出目标，再继续链 `.with_stores([...])`。
如果要替换默认 enqueue admission 去重实现，再继续链 `.with_dedup(...)`。
如果要替换默认 robots policy，再继续链 `.with_robots(...)`。
如果要监听全部 runtime 事件，再继续链 `.with_signal_listener(...)`。
如果只想监听部分 signal kind，再用 `.with_signal_listener_for([...], ...)`。
如果要挂扩展，再继续链 `.with_extension(...)` 或 `.with_extension_for([...], ...)`。

推荐这样理解选择顺序：

- 最简单场景：`Engine::new()`
- 单机内存调度：`scheduler::Memory`
- 单机 durable 调度：`scheduler::Sqlite`
- 多 worker / 多机共享调度：`scheduler::Redis`
- 自定义后端：实现 `scheduler::Scheduler`

最常见的几种接法：

```rust
use halo_spider::dedup;
use halo_spider::download::{Browser, Http};
use halo_spider::engine::Engine;
use halo_spider::robots;
use halo_spider::scheduler;
use halo_spider::store;

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

// 5. 显式替换 dedup
let engine = Engine::new().with_dedup(
    dedup::Memory::new().with_keys([dedup::Key::Url, dedup::Key::Method]),
);

// 6. 禁用 dedup
let engine = Engine::new().with_dedup(dedup::Noop);

// 7. 替换 robots
let engine = Engine::new().with_robots(robots::Memory::default());

// 8. memory scheduler + file checkpoint
let engine = Engine::new().with_checkpoint(scheduler::checkpoint::File::default());

// 9. 单机 durable scheduler
let engine = Engine::from_parts(
    scheduler::Sqlite::new("output/scheduler.db", "jobs:news"),
    Http::default(),
    Browser::default(),
);

// 10. 共享 durable scheduler
let engine = Engine::from_parts(
    scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:news"),
    Http::default(),
    Browser::default(),
);

// 11. 保留默认 default store，再额外挂多个可路由 store
let engine = Engine::new().with_stores([
    store::StoreEntry::new(
        "article_db",
        store::Sqlite::new("output/items.db").with_table("articles"),
    ),
    store::StoreEntry::new(
        "article_file",
        store::File::new("output/articles.jsonl"),
    ),
]);
```

几个重要边界：

- `Engine::new()` 默认就是 `scheduler::Memory + download::Http + download::Browser + enqueue admission dedup::Memory + robots::Memory + default store("default") -> store::File`
- `with_store(store)` 是替换默认 `default` store
- `with_stores([...])` 是额外注册可被 rules `output.sinks` 路由命中的 store
- rules 不直接创建 store 实例；`output.sinks` 只是在 rules 编译结果里保留目标 store 名称，真正解析成 store 实例发生在 engine 构建 step 运行时时
- 当前不再单独引入 `with_queue(...)`；任务排队、`ready / delayed / inflight` 流转与恢复边界统一收口在 `scheduler`
- 如果你是手动往引擎里塞 request，优先用 `engine.enqueue(request).await?`；直接调 `engine.scheduler.enqueue(...)` 会绕过 enqueue admission 边界，至少包括 dedup 这类 `before_enqueue` 逻辑
- `checkpoint` 是静态快照恢复边界，不替代 durable scheduler 的 runtime reclaim

## 代码流程架构图

当前代码可以按下面这张图理解：

```text
+-------------------+                         +----------------------+
| Spider 实现        |                         | Rules DSL / JSON     |
| - name()          |                         | - spider             |
| - start_requests()|                         | - engine             |
| - parse()/call()  |                         | - seeds              |
| - validator()     |                         | - steps              |
+---------+---------+                         +----------+-----------+
          |                                              |
          | spider.rules()                               |
          |--------------------------------------------->|
          |                                              v
          |                                     rules::load(...)
          |                           normalize -> validate -> compile
          |                                              |
          |                                              v
          |                                Compiled { seeds, steps, sinks }
          |                                              |
          +-------------------------+--------------------+
                                    |
                                    v
+--------------------------------------------------------------------------+
| Engine::run                                                               |
| - open pipeline                                                           |
| - open stores                                                             |
| - build_step_executes(compiled, spider.validator())                       |
| - enqueue_start_requests(...)                                             |
| - loop { claim task -> execute task -> commit scheduler result }          |
| - 直到 stop()/shutdown 才退出，不会因队列空自动结束                         |
+--------------------------+-----------------------------------------------+
                           |
                           v
                +---------------------------+
                | StepExecute               |
                | - chain                   |
                | - stores                  |
                | - step_validator          |
                +-------------+-------------+
                              |
                              v
                +---------------------------+
                | TaskExecutor::run         |
                +-------------+-------------+
                              |
                              v
    before_enqueue / after_enqueue   -> Scheduler enqueue / claim
                              |
                              v
    before_download
         |
         +--> Downloader
               - http
               - browser
         |
         +--> after_download
         +--> download_error
                              |
                              v
    before_parse
         |
         +--> spider.dispatch(...)
               - request.callback => spider.call(...)
               - step.callback    => spider.call(...)
               - 否则             => rules::apply(...)
         |
         +--> after_parse / parse_error
                              |
                              v
    process_spider_output
         |
         +--> before_item / after_item
         +--> pipeline.process(item)
         +--> step_validator
         +--> stores.batch_write(items)
         +--> callback 产出的 follow requests -> 回到 enqueue
```

几个关键点：

- `Engine` 只持有一份 engine 级 middleware、pipeline、store 注册表、scheduler、downloaders。
- `StepExecute` 是 step 运行时快照，专门收口这个 step 的 middleware、stores、validator。
- rules 不直接实例化 store；rules 只产出 `output.sinks` 名称，engine 在 `build_step_executes(...)` 阶段把它解析成目标 store 实例列表。
- middleware 实际执行永远是两层：先 engine middleware，再 step middleware。
- `Response::urljoin(...)`、`Response::follow(...)`、`Response::follow_all(...)` 只负责从当前响应构造子请求；真正的 `dedup`、下载前 middleware 与重试 middleware 仍然在 request + middleware 执行链里解析。

## 单条 Request 时序

单条 request 进入引擎后，真实执行顺序可以按下面理解：

```text
Request
  |
  v
before_enqueue
  |- engine middleware
  `- step middleware
  |
  +-- Enqueue::Drop ------> 丢弃，不进入 scheduler
  |
  v
scheduler.enqueue
  |
  v
after_enqueue
  |
  v
scheduler.claim
  |
  v
before_download
  |- engine middleware
  `- step middleware
  |
  +-- Download::Drop  -----> complete/drop
  +-- Download::Delay -----> requeue delayed task
  |
  v
download(http/browser)
  |
  +-- error
  |    |
  |    v
  |  download_error
  |    |- engine middleware
  |    `- step middleware
  |    |
  |    +-- Retry ---------> complete_and_enqueue retry task
  |    +-- Drop ----------> complete/drop
  |    `-- Continue ------> request.errback? / task error
  |
  `-- response
       |
       v
     after_download
       |- engine middleware
       `- step middleware
       |
       +-- Retry ---------> complete_and_enqueue retry task
       +-- Delay ---------> complete_and_enqueue delayed task
       +-- Drop ----------> complete/drop
       |
       v
     before_parse
       |- engine middleware
       `- step middleware
       |
       v
     spider.dispatch
       |- request.callback -> spider.call(...)
       |- step.callback    -> spider.call(...)
       `- rules step       -> rules::apply(...)
       |
       +-- parse error
       |    |
       |    v
       |  parse_error
       |    |- engine middleware
       |    `- step middleware
       |    |
       |    +-- Drop -----> complete/drop
       |    `-- Continue -> task error
       |
       `-- callback result
             - Item / Vec<Item>
             - Request / Vec<Request>
             - (items, requests)
             |
             v
           before_item
             |
             v
           pipeline.process
             |
             v
           step_validator
             |
             v
           after_item
             |
             v
           stores.batch_write
             |
             v
           follow requests
             |
             `--> 回到 before_enqueue
```

这里可以把 flow 的职责也一起记住：

- `Enqueue` 只控制“能不能进队列”。
- `Download` 负责 `Continue / Drop / Delay / Retry`，因为下载阶段确实需要这几种分支。
- `Parse` 只控制“是否继续解析结果”。
- `Item` 只控制“item 是否继续进入 pipeline / store”。

## Durable Scheduler 语义

当前 `scheduler::Memory`、`scheduler::Sqlite`、`scheduler::Redis` 已经统一了核心模型：

- 任务状态统一为 `ready / delayed / inflight`
- 排序统一支持 `priority / depth`
- 读入口统一为 `checkpoint() / counts() / snapshot() / scopes() / snapshots() / overview()`
- 控制入口统一为 `pause_scope() / resume_scope() / release_scope() / purge_scope()`
- 吞吐入口统一为 `take_batch_ready(limit)`、`complete_batch(...)`、`requeue_batch(...)`、`complete_and_enqueue_batch(...)`

运维上建议优先这样看：

- 看单个 scope 即时状态：`snapshot()`
- 看多个 scope 聚合：`overview()` / `overview_with_prefix(...)`
- 看某个 worker 当前 inflight：`snapshot.inflight_tasks`
- 看 worker 心跳和 lease：`snapshot.workers`

例如：

```rust
use halo_spider::scheduler::{self, Control, Scheduler};

let scheduler = scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:news")
    .with_worker(scheduler::Worker::new("news-worker-a"));

let snapshot = scheduler.snapshot().await?;
let overview = scheduler.overview().await?;

scheduler.pause_scope("jobs:news").await?;
scheduler.release_scope("jobs:news").await?;
scheduler.resume_scope("jobs:news").await?;
```

当前明确的提交边界：

- engine 按 `store(s) -> scheduler complete/complete_and_enqueue` 顺序提交结果
- 这里不是跨 `store(s) / scheduler` 的分布式事务
- 如果任一目标 `store` 失败，这轮任务不会被静默标记为完成
- 如果全部 `store` 成功而 `scheduler resolve` 失败，这条边界按 at-least-once 理解

如果要深入了解：

- 分布式运行说明见 [distributed_scheduler.md](../distributed_scheduler.md)
- 自定义后端骨架见 [custom_scheduler_backend.md](../custom_scheduler_backend.md)
- 运维侧看板/巡检/控制思路见 [operations.md](../operations.md)

## Runtime 观测

当前 runtime 观测分成四层：

- `stats`：看单个 engine 实例生命周期内的累计计数
- `signals / extensions`：监听生命周期和运行时事件
- `trace`：排查单个 request、store 提交和 scheduler resolve 边界
- `telemetry`：把 `stats + scheduler runtime` 统一导出到 `Collector / File / Prometheus / OpenTelemetry`

最小用法：

```rust
use halo_spider::engine::Engine;
use halo_spider::signals;

let engine = Engine::new()
    .with_signal_listener(|event| async move {
        println!("signal: {:?}", event.kind);
    })
    .with_signal_listener_for([signals::Kind::SchedulerEvent], |event| async move {
        println!("scheduler event: {:?}", event.scheduler_event);
    });
```

如果你想统一导出 telemetry：

```rust
use halo_spider::engine::Engine;

let collector = halo_spider::telemetry::Collector::default();
let engine = Engine::new().with_telemetry(collector.clone());
```

如果你想接 Prometheus / OpenTelemetry：

```rust
use halo_spider::engine::Engine;

let prometheus = halo_spider::telemetry::Prometheus::default();
let engine = Engine::new().with_telemetry(prometheus.clone());

let otel = halo_spider::telemetry::OpenTelemetry::builder(
    "http://127.0.0.1:4318/v1/metrics",
)
.with_service_name("my_spider")
.build()?;
let engine = Engine::new().with_telemetry(otel.clone());
otel.shutdown()?;
```

更完整的运行流转图、`Success / Retry / Drop / Error / LeaseLost` 分支说明，以及排障建议见 [capabilities.md](../capabilities.md)。

## Request Middleware 与排序

现在 request 级运行策略都按具体 middleware 展开，不再保留组级 key。

- 下载前控制直接使用 `concurrency`、`interval`、`rate_limit`、`auto_throttle`
- 重试直接使用 `retry_by_status`、`retry_by_error`

所以代码里更推荐这样写：

```rust
let request = Request::new("https://example.com/detail")
    .with_retry_by_status(retry_cfg, 200)
    .with_retry_by_error(retry_cfg, 210)
    .with_interval(interval_cfg, 120)
    .with_rate_limit(rate_cfg, 130);
```

如果这条 request 想显式跳过某个 middleware，也直接按 middleware 名称写：

```rust
use halo_spider::middleware::{DEDUP, RETRY_BY_STATUS};

let request = Request::new("https://example.com/detail")
    .skip([DEDUP, RETRY_BY_STATUS]);
```

rules DSL 侧与它对应的写法是：

```yaml
request:
  url: "https://example.com/detail"
  skip:
    - "dedup"
    - "retry_by_status"
```

### `Config` 是什么

`Config::with_request_middleware(...)`、step 默认 middleware、以及 engine 直接挂载 middleware 时，用的都是同一个 `middleware::Config`：

```rust
middleware::Config {
    enabled: true,
    stage: Stage::Download,
    order: 115,
    options: BTreeMap::new(),
}
```

字段含义：

- `enabled`
  - 默认是否启用这条 middleware
- `stage`
  - 生效阶段，当前主要是 `Stage::Download` 或 `Stage::Spider`
- `order`
  - 默认执行顺序，数字越小越早执行
- `options`
  - 默认参数，具体字段由各 middleware 自己解释

可以直接把它理解成：

- `Config` 不是 request 本身
- `Config` 是“这条 middleware 默认怎么挂”

### 排序优先级

当前顺序规则固定是：

1. 如果 request 自己显式写了 order，优先用 request 的
2. 否则用 step 默认里的 `Config.order`
3. 如果 step 没配，再回退到 engine / settings 默认里的 `Config.order`

所以：

- `Config.order` 是默认顺序
- `Request::with_xxx(..., order)` 是单次覆盖顺序

### 自定义 middleware 的排序

自定义 middleware 和内置 middleware 走的是同一套顺序逻辑。

推荐接法：

```rust
use halo_spider::engine::Engine;
use halo_spider::middleware::Stage;
use halo_spider::request::Request;
use halo_spider::settings::Config;
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

let request = Request::new("https://example.com/detail")
    .with_middleware_options_ordered(CUSTOM_HEADER, header_cfg, 118);
```

这段代码表示：

- `custom_header` 默认挂在下载阶段
- 默认顺序是 `115`
- 这一次 request 临时把顺序改成 `118`

如果某一条 request 不想让它生效，也可以直接：

```rust
let request = Request::new("https://example.com/detail")
    .skip([CUSTOM_HEADER]);
```

## 并发控制配置

最小并发控制：

```rust
let config = Config::default()
    .with_concurrent_requests(16)
    .with_concurrent_requests_per_domain(8)
    .with_download_delay(jiff::SignedDuration::from_millis(200));
```

如果你想改成按站点反馈动态调速，可以开启最小 `AutoThrottle`：

```rust
let config = Config::default()
    .with_auto_throttle(true)
    .with_download_delay(jiff::SignedDuration::from_millis(200))
    .with_auto_throttle_target_concurrency(1.0)
    .with_auto_throttle_max_delay(jiff::SignedDuration::from_secs(5));
```

如果你想让同一个 HTTP `GET` 请求自动走最小条件请求缓存，也可以直接开启：

```rust
use halo_spider::engine::Engine;
use halo_spider::settings::Config;

let engine = Engine::new().with_config(
    Config::default()
        .with_http_cache(true)
        .with_http_cache_ttl(jiff::SignedDuration::from_hours(12))
        .with_http_cache_strategy(halo_spider::middleware::http_cache::Strategy::Response),
);
```

这层 `http_cache` 当前边界：

- 只作用于 HTTP `GET`
- 默认 backend 是进程内 `middleware::http_cache::Memory`
- 也可以通过 `Config::with_http_cache_file(...)` 或 `HttpCache::with_cache(...)` 换成文件或自定义 backend
- 当前支持 `validators` 和 `response` 两种策略
- 当前仍然不做 `Cache-Control` / `Expires` / `Vary` 这类更完整 HTTP 缓存语义

对应示例：

- `examples/concurrency_control.rs`
- `examples/http_cache.rs`
- `examples/custom_http_cache.rs`
