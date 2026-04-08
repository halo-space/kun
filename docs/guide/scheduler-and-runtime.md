# Scheduler 与 Runtime

[返回使用手册](../guide.md)

## Scheduler 选择

如果默认组件足够，直接用 `Engine::new()`。
如果想保留默认 scheduler、只替换下载器，优先用 `.with_http(...)` / `.with_browser(...)`。
如果要连 `scheduler` 一起自定义，再用 `Engine::from_parts(scheduler, http, browser)`。
如果要替换默认去重实现，再继续链 `.with_dedup(...)`。
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
```

几个重要边界：

- `Engine::new()` 默认就是 `scheduler::Memory + download::Http + download::Browser + dedup::Memory + robots::Memory + store::File`
- 当前不再单独引入 `with_queue(...)`；任务排队、`ready / delayed / inflight` 流转与恢复边界统一收口在 `scheduler`
- 如果你是手动往引擎里塞 request，优先用 `engine.enqueue(request).await?`；直接调 `engine.scheduler.enqueue(...)` 会绕过 dedup
- `checkpoint` 是静态快照恢复边界，不替代 durable scheduler 的 runtime reclaim

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

- engine 按 `store -> scheduler complete/complete_and_enqueue` 顺序提交结果
- 这里不是跨 `store / scheduler` 的分布式事务
- 如果 `store` 失败，这轮任务不会被静默标记为完成
- 如果 `store` 成功而 `scheduler resolve` 失败，这条边界按 at-least-once 理解

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

## 并发控制配置

最小并发控制：

```rust
let settings = Settings::default()
    .with_concurrent_requests(16)
    .with_concurrent_requests_per_domain(8)
    .with_connection_pool_size(100)
    .with_download_delay(jiff::SignedDuration::from_millis(200));
```

如果你想改成按站点反馈动态调速，可以开启最小 `AutoThrottle`：

```rust
let settings = Settings::default()
    .with_auto_throttle(true)
    .with_download_delay(jiff::SignedDuration::from_millis(200))
    .with_auto_throttle_target_concurrency(1.0)
    .with_auto_throttle_max_delay(jiff::SignedDuration::from_secs(5));
```

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

这层 `http_cache` 当前边界：

- 只作用于 HTTP `GET`
- 默认 backend 是进程内 `middleware::http_cache::Memory`
- 也可以通过 `Settings::with_http_cache_file(...)` 或 `HttpCache::with_cache(...)` 换成文件或自定义 backend
- 当前支持 `validators` 和 `response` 两种策略
- 当前仍然不做 `Cache-Control` / `Expires` / `Vary` 这类更完整 HTTP 缓存语义

对应示例：

- `examples/concurrency_control.rs`
- `examples/http_cache.rs`
- `examples/custom_http_cache.rs`
