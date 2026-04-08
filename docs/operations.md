# 运维与观测指南

这份文档专门解释 `kun` 当前和运维 / 观测有关的能力怎么用。

它不再讨论 scheduler 的底层流转语义，而是回答这几个更实际的问题：

- 现在线上运行时，到底有哪些数据可以拿
- 这些数据怎么持续采集、落盘、聚合
- 跨多个 job / scope 怎么看
- `pause / resume / release / purge` 这类控制动作怎么做

先给一个结论：

- `kun` 现在已经有完整的“可编程运维原语”
- 还没有内置“一站式 daemon / dashboard / admin service”
- 所以当前最自然的做法是：基于现有 API 自己封一层 CLI / HTTP / 定时巡检任务

## 三层模型

如果从运维角度看，当前能力可以分成三层：

### 数据层

负责回答“现在有哪些数据源可以拿”。

- `Engine::stats()`：单个 engine 实例生命周期内的累计计数
- `signals`：engine 内部最原始、最语义化的运行事件流
- `scheduler runtime events`：scheduler 后端的 runtime 事件
- `scheduler.snapshot()` / `snapshots_with_prefix()` / `overview_with_prefix()`：durable scheduler 的即时状态读取
- `telemetry` exporter：把 `stats + scheduler runtime` 持续导出到内存、文件、Prometheus、OTel

### 视图层

负责回答“这些数据怎么查、怎么展示”。

- `Collector::snapshot()`：内存快照，适合直接给 HTTP API / 页面返回
- `Prometheus::render()`：适合被 scrape
- `snapshot / overview / snapshots_with_prefix`：适合跨 job / scope 做巡检视图

### 控制层

负责回答“发现问题后怎么操作”。

- `pause_scope()`
- `resume_scope()`
- `release_scope()`
- `purge_scope()`
- `release_inflight()`

也就是说，当前没有内置 daemon，但“数据源、视图原语、控制原语”都已经有了。

## 1. 数据层

### 1.1 读取单个 engine 的累计计数

`Engine::stats()` 返回的是当前 engine 实例从启动到现在的累计计数。

```rust
let stats = engine.stats();

println!(
    "request={} response={} item={} retry={} store_error={}",
    stats.request_count,
    stats.response_count,
    stats.item_count,
    stats.retry_count,
    stats.store_error_count,
);
```

适合看：

- 这个 engine 本轮到底跑了多少请求
- 重试了多少次
- 最终写出了多少 item
- 有没有明显的 store 错误

不适合看：

- 某个 durable scheduler backend 的全局 backlog
- 多个 worker / 多个 scope 的即时分布

因为这份计数是“单个 engine 生命周期内的累计值”，不是后端全局状态。

### 1.2 读取 durable scheduler 的即时状态

如果你关心的是 backlog、inflight、worker heartbeat、stale lease、跨 scope 聚合这些运维信息，优先读 scheduler 的状态接口。

```rust
use halo_spider::scheduler::{self, Scheduler};

let scheduler = scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:news");

let snapshot = scheduler.snapshot().await?;
println!("scope={}", snapshot.scope);
println!("ready={}", snapshot.counts.ready);
println!("delayed={}", snapshot.counts.delayed);
println!("inflight={}", snapshot.counts.inflight);
println!("workers={}", snapshot.workers.len());
```

如果要跨多个 job / scope 看：

```rust
use halo_spider::scheduler::{self, Scheduler};

let ops = scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:ops");

let scopes = ops.scopes_with_prefix("jobs:").await?;
println!("visible scopes: {:?}", scopes);

let overview = ops.overview_with_prefix("jobs:").await?;
println!(
    "scope_count={} pending_scope_count={} worker_count={}",
    overview.scope_count,
    overview.pending_scope_count,
    overview.worker_count,
);

for snapshot in ops.snapshots_with_prefix("jobs:").await? {
    println!(
        "scope={} paused={} ready={} delayed={} inflight={}",
        snapshot.scope,
        snapshot.is_paused,
        snapshot.counts.ready,
        snapshot.counts.delayed,
        snapshot.counts.inflight,
    );
}
```

这组接口是当前最直接的“跨 job 巡检数据源”。

### 1.3 持续导出统一 telemetry 流

如果你想把 `stats + scheduler runtime` 持续落盘或送到外部监控系统，优先用 `Engine::with_telemetry(...)`。

最常用的是把多个 exporter 扇出到一起：

```rust
use halo_spider::engine::Engine;
use halo_spider::telemetry;

let collector = telemetry::Collector::default();
let file = telemetry::File::new("output/telemetry.jsonl")?;
let prometheus = telemetry::Prometheus::default();

let telemetry = telemetry::Fanout::new()
    .with_exporter(collector.clone())
    .with_exporter(file.clone())
    .with_exporter(prometheus.clone());

let engine = Engine::new().with_telemetry(telemetry);
```

这三种 exporter 的定位分别是：

- `Collector`：内存聚合，适合 API / dashboard 直接读取
- `File`：追加写 JSONL，适合最小持久化事件流
- `Prometheus`：适合 pull scrape

其中 `telemetry::File` 当前最接近“最小持久化事件总线”的内置实现。
它会把每条 telemetry event 追加写成一行 JSON，适合：

- 本地留底
- 被 `tail` / `fluent-bit` / `vector` 这类 agent 转发
- 后续再汇总进 ELK、ClickHouse、对象存储或别的分析链路

如果你想直接推到 OTel Collector，也可以这样接：

```rust
use halo_spider::telemetry;

let otel = telemetry::OpenTelemetry::builder(
    "http://127.0.0.1:4318/v1/metrics",
)
.with_service_name("kun-worker")
.build()?;

let engine = halo_spider::engine::Engine::new().with_telemetry(otel.clone());

// 进程结束前做一次最终 flush
otel.shutdown()?;
```

### 1.4 如果你需要最原始的 signal 事件流

`telemetry` 当前统一导出的是：

- `stats`
- `scheduler runtime`

如果你还想持久化这些更语义化的 engine 事件：

- `spider_opened`
- `spider_closed`
- `request_scheduled`
- `response_received`
- `item_scraped`
- `spider_error`

那就应该自己实现一个 `signals::Listener`。

示意写法如下：

```rust,ignore
use halo_spider::engine::Engine;
use halo_spider::future::BoxFuture;
use halo_spider::signals::{Listener, Signal};

#[derive(Clone)]
struct JsonlSignalSink;

impl Listener for JsonlSignalSink {
    fn on_signal<'a>(&'a self, signal: &'a Signal) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            // 这里可以写文件、发 Kafka、写 Redis Stream、写数据库等
            append_json_line("output/signals.jsonl", signal).await;
        })
    }
}

let engine = Engine::new().with_signal_listener(JsonlSignalSink);
```

也就是说：

- 想持久化 `stats + scheduler runtime`：优先 `with_telemetry(...)`
- 想持久化更完整的 engine 语义事件：自己挂 `with_signal_listener(...)`

### 1.5 如果你需要 scheduler 后端自己的 runtime 事件

`Engine::with_telemetry(...)` 主要覆盖 engine 主流程参与到的 scheduler runtime。

如果你还想观测这些后端级动作：

- `reclaim`
- `release_inflight`
- `close`

那就可以显式包一层 `scheduler::Observed`：

```rust
use halo_spider::scheduler;

let metrics = scheduler::MetricsReporter::new();

let scheduler = scheduler::Observed::new(
    scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:news"),
)
.with_reporter(metrics.clone());

let snapshot = metrics.snapshot();
println!("claimed_total={}", snapshot.totals.claimed_total);
println!("reclaimed_total={}", snapshot.totals.reclaimed_total);
```

如果你不想只要内存计数，也可以把 exporter 直接挂到 `Observed` 上：

```rust
use halo_spider::scheduler;
use halo_spider::telemetry;

let scheduler = scheduler::Observed::new(
    scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:news"),
)
.with_exporter(telemetry::File::new("output/scheduler-runtime.jsonl")?);
```

## 2. 视图层

### 2.1 单个 scope 的视图

最直接的视图就是 `snapshot()`。

- 当前 `ready / delayed / inflight` 各多少
- 当前有哪些活跃 worker
- 每个 inflight task 的 `task_id / url / worker_id / lease_id / deadline`

适合：

- 定位某个具体 job 是否卡住
- 看某个 worker 是否还活着
- 看某批 inflight task 有没有 stale 的迹象

### 2.2 跨 job / scope 的视图

如果你要做一个简单的巡检页面，最常用的是这三个：

- `scopes_with_prefix(prefix)`
- `snapshots_with_prefix(prefix)`
- `overview_with_prefix(prefix)`

建议分工如下：

- 列表页：优先 `overview_with_prefix(...)`
- 详情页：点进某个 scope 再读 `snapshot()`
- 批量巡检页：直接读 `snapshots_with_prefix(...)`

一个很常见的组合是：

```rust
use halo_spider::scheduler::{self, Scheduler};

let ops = scheduler::Sqlite::new("output/scheduler.db", "jobs:ops");
let overview = ops.overview_with_prefix("jobs:").await?;

println!("scope_count={}", overview.scope_count);
println!("pending_scope_count={}", overview.pending_scope_count);
println!("stale_scope_count={}", overview.stale_scope_count);
println!("worker_count={}", overview.worker_count);
```

### 2.3 用 `Collector` 做 API / Dashboard 背后的内存视图

如果你已经给 engine 挂了 `telemetry::Collector`，就可以把它当成一个内存态视图源：

```rust
use halo_spider::telemetry;

let collector = telemetry::Collector::default();
let engine = halo_spider::engine::Engine::new().with_telemetry(collector.clone());

let snapshot = collector.snapshot();
println!("request_count={}", snapshot.stats.request_count);
println!(
    "scheduler_claimed_total={}",
    snapshot.scheduler.totals.claimed_total
);
println!("recent_events={}", snapshot.recent_events.len());
```

这非常适合：

- 一个本地 admin API
- 一个进程内 dashboard
- 单实例 worker 的 `/metrics/debug` 页面

### 2.4 用 `Prometheus` 做 pull 风格监控

如果你想直接给 Prometheus scrape：

```rust
use halo_spider::telemetry;

let prometheus = telemetry::Prometheus::default();
let engine = halo_spider::engine::Engine::new().with_telemetry(prometheus.clone());

let body = prometheus.render();
println!("{body}");
```

通常做法是：

- 进程内暴露一个 `/metrics`
- handler 里返回 `prometheus.render()`

当前库没有内置 HTTP server，但 exporter 本身已经给了。

## 3. 控制层

### 3.1 跨 scope 运维控制

当前统一运维控制入口就是 `scheduler::Control`。

```rust
use halo_spider::scheduler::{self, Control, Scheduler};

let ops = scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:ops")
    .with_worker(scheduler::Worker::new("ops-worker"));

let scopes = ops.scopes_with_prefix("jobs:").await?;
println!("visible scopes: {:?}", scopes);

let changed = ops.pause_scope("jobs:news").await?;
println!("pause changed: {changed}");

let snapshot = ops
    .snapshots_with_prefix("jobs:")
    .await?
    .into_iter()
    .find(|snapshot| snapshot.scope == "jobs:news")
    .unwrap();
println!("news paused: {}", snapshot.is_paused);

let released = ops.release_scope("jobs:news").await?;
println!("released inflight tasks: {released}");

let removed = ops.purge_scope("jobs:news").await?;
println!("purged counts: {:?}", removed);

ops.resume_scope("jobs:news").await?;
```

这段代码本身就已经可以是：

- 一个 CLI 命令
- 一个 cron / 定时巡检任务
- 一个 HTTP admin handler
- 一个内部运维面板后的控制 API

### 3.2 每个动作分别是什么意思

- `pause_scope(scope)`：暂停这个 scope 后续的 claim / 调度流转
- `resume_scope(scope)`：恢复这个 scope 的正常调度
- `release_scope(scope)`：把这个 scope 当前 inflight 的任务主动交回可执行队列
- `purge_scope(scope)`：清空这个 scope 当前的任务 / worker 运行态，是明确的破坏性操作
- `release_inflight()`：只交回“当前 worker 自己手里的 inflight task”，更适合优雅下线

### 3.3 推荐的安全操作顺序

如果你在做人工运维，比较稳妥的顺序通常是：

1. `pause_scope(scope)`
2. `snapshot()` / `snapshots_with_prefix(...)` 看清当前状态
3. 如果只是 worker 宕掉或想重新分配任务，用 `release_scope(scope)`
4. 只有在确认要彻底放弃这个 scope 时，再用 `purge_scope(scope)`
5. 最后 `resume_scope(scope)`

也就是说：

- `release_scope` 更像“交回正在执行的活”
- `purge_scope` 更像“清空整个 job 当前状态”

## 4. 视图和控制怎么组合

最常见的几种组合模式如下。

### 4.1 最小本地运维脚本

适合：

- 单机部署
- 本地排障
- 偶尔人工操作

推荐组合：

- `overview_with_prefix(...)` 看全局
- `snapshot()` 看单个 scope
- `pause_scope / release_scope / resume_scope` 做人工控制

### 4.2 持续留历史

适合：

- 想保留一份最小历史数据
- 后续再异步导入分析系统

推荐组合：

- `Engine::with_telemetry(telemetry::File::new(...))`
- 如果还要更多语义事件，再加 `with_signal_listener(...)`

### 4.3 自建运维 API / 面板

适合：

- 内部控制台
- 自己做一个 `/ops/scheduler/*` 服务

推荐组合：

- 视图：`Collector::snapshot()`、`overview_with_prefix(...)`、`snapshots_with_prefix(...)`
- 控制：`pause_scope / resume_scope / release_scope / purge_scope`

### 4.4 接 Prometheus / OTel

适合：

- 已有监控基础设施
- 希望统一接到现成的告警系统

推荐组合：

- `telemetry::Prometheus`
- `telemetry::OpenTelemetry`
- 如有需要再配 `telemetry::Fanout`

## 5. 当前没有内置的部分

当前没有内置的是这些“产品层”东西：

- 自带的跨 job Web 仪表盘
- 自带的常驻巡检 daemon
- 自带的 HTTP admin service
- 自带的全量 signals 持久化总线

但这不表示底层能力没准备好。

更准确地说，当前状态是：

- 数据源：已内置
- 视图原语：已内置
- 控制原语：已内置
- 一站式运维产品：还没有内置实现

## 6. 推荐阅读

- 能力总览：[capabilities.md](./capabilities.md)
- 分布式 scheduler 用法：[distributed_scheduler.md](./distributed_scheduler.md)
- 自定义 scheduler backend：[custom_scheduler_backend.md](./custom_scheduler_backend.md)
- telemetry 示例：[`examples/telemetry.rs`](../examples/telemetry.rs)
- 自定义 scheduler 示例：[`examples/custom_scheduler.rs`](../examples/custom_scheduler.rs)
- 最小 middleware plugin 示例：[`examples/middleware_plugin.rs`](../examples/middleware_plugin.rs)
