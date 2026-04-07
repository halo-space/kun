# Distributed Scheduler

`scheduler::Redis` 是运行时调度器，不是 checkpoint。

只要某个 `scheduler::Redis` 真正参与过 enqueue / claim / snapshot / counts 这类访问，
它的 namespace 就会自动登记到同一个 Redis 里的 durable scheduler registry。

在统一 scheduler 抽象里，这里的 Redis `namespace` 就是一个 `scope`。

最小可以这样理解：

- `namespace`
  - 一份共享任务池
  - 通常对应一个具体站点、一个具体爬虫任务，或者一个独立 job
- `worker_id`
  - 一个实际运行中的 worker 进程实例
  - 多台机器或多个进程一起跑时，每个实例都应该不一样
- `lease`
  - 某个 worker 成功 claim 一条 task 后拿到的执行凭证
  - 后续 `heartbeat / complete / requeue` 都要带着这份 lease
- `heartbeat`
  - task 运行时间比较长时，engine 会定期续租
  - 这样不会因为 lease timeout 到期，被别的 worker 提前回收

## 最小配置

```rust
use halo_spider::engine::Engine;
use halo_spider::scheduler;

let engine = Engine::new().with_scheduler(
    scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:news").with_worker(
        scheduler::Worker::new("news-worker-a")
            .with_lease_timeout(jiff::SignedDuration::from_secs(30))
            .with_heartbeat_interval(jiff::SignedDuration::from_secs(10)),
    ),
);
```

## 多 worker 怎么跑

同一个站点或 job：

- 所有 worker 用同一个 `namespace`
- 每个 worker 用不同的 `worker_id`
- 它们会共享 ready / delayed / inflight 这套 Redis 状态

例如两个实例一起跑同一个 job：

```rust
let worker_a = scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:news")
    .with_worker(scheduler::Worker::new("news-worker-a"));

let worker_b = scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:news")
    .with_worker(scheduler::Worker::new("news-worker-b"));
```

这样两个 worker 会从同一个任务池里 claim task，但不会重复 claim 同一条 ready task。

## 读取 scope 运行时快照

如果你想直接看某个 scope 当前的 durable scheduler 状态，可以通过统一的
`Scheduler::snapshot()` 读接口：

```rust
use halo_spider::scheduler::{self, Scheduler};

let scheduler = scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:news")
    .with_worker(scheduler::Worker::new("ops-reader"));

let snapshot = scheduler.snapshot().await?;

println!("{:?}", snapshot.counts);
println!("workers: {:?}", snapshot.worker_ids);
println!("active leases: {}", snapshot.active_lease_count);
println!("reclaimed total: {}", snapshot.reclaimed_total);
println!("reclaimed in refresh: {}", snapshot.reclaimed_in_refresh);
for worker in &snapshot.workers {
    println!(
        "worker={} last_seen={:?} stale={} inflight={} next_deadline={:?}",
        worker.worker_id,
        worker.last_seen,
        worker.is_stale,
        worker.inflight_count,
        worker.next_deadline
    );
}
for task in &snapshot.inflight_tasks {
    println!(
        "inflight task={} url={} worker={:?} lease={:?} deadline={:?}",
        task.task_id.as_str(),
        task.url,
        task.worker_id,
        task.lease_id,
        task.deadline
    );
}
```

这里有两个边界要区分：

- `snapshot()` 读的是某个 namespace 当前这一刻的运行时状态
- `Engine::stats()` 读的是单个 engine 实例生命周期内的累计计数
- `snapshot.workers` 是 worker 级运行态视图；`snapshot.worker_ids` 只是聚合后的 worker id 集合
- `snapshot()` / `counts()` / `checkpoint()` 不会把当前调用方登记成活跃 worker
- 单纯 `enqueue()` 也不会创建 worker runtime
- 但如果某个 worker 已经参与过 lease 生命周期，后续空轮询 `take_ready()` 仍会刷新它的 `last_seen`，避免 idle worker 被误判成 stale

## 跨 job 运维怎么读

如果同一个 Redis 里跑了多个 scope，可以先按前缀发现它们，再先读聚合概览，
再按需批量读取各自快照：

```rust
use halo_spider::scheduler::{self, Scheduler};

let scheduler = scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:ops")
    .with_worker(scheduler::Worker::new("ops-reader"));

let scopes = scheduler.scopes_with_prefix("jobs:").await?;
println!("registered scopes: {:?}", scopes);

let overview = scheduler.overview_with_prefix("jobs:").await?;
println!("scope_count: {}", overview.scope_count);
println!("pending_scope_count: {}", overview.pending_scope_count);
println!("stale_scope_count: {}", overview.stale_scope_count);
println!("counts: {:?}", overview.counts);
println!("worker_count: {}", overview.worker_count);
println!("active_lease_count: {}", overview.active_lease_count);
println!("reclaimed_total: {}", overview.reclaimed_total);

let snapshots = scheduler.snapshots_with_prefix("jobs:").await?;

for snapshot in snapshots {
    println!("scope: {}", snapshot.scope);
    println!("counts: {:?}", snapshot.counts);
    println!("workers: {:?}", snapshot.worker_ids);
    println!("reclaimed_total: {}", snapshot.reclaimed_total);
    for worker in &snapshot.workers {
        println!(
            "  worker={} stale={} inflight={}",
            worker.worker_id,
            worker.is_stale,
            worker.inflight_count
        );
    }
    for task in &snapshot.inflight_tasks {
        println!(
            "  inflight task={} worker={:?} lease={:?}",
            task.task_id.as_str(),
            task.worker_id,
            task.lease_id
        );
    }
}
```

这层边界也要明确：

- `scopes_with_prefix(...)` / `snapshots_with_prefix(...)` / `overview_with_prefix(...)` 现在属于统一的 `scheduler::Scheduler` 读能力
- 对 `scheduler::Redis` 来说，它们会读 Redis 里共享 registry 下的多个 scope；对本地 `scheduler::Memory`，默认只会返回当前 scope
- `overview_with_prefix(...)` 底层也是从 `snapshots_with_prefix(...)` 聚合出来的，所以它同样看到的是“这次读取后”的即时状态
- `snapshots_with_prefix(...)` 读取时会顺带刷新各 scope 的 stale reclaim，所以它看到的是“这次读取后”的即时状态

## 怎么看 worker 运行态

`snapshot.workers` 里的每一项都代表一个当前 namespace 下见过的 worker：

- `worker_id`
  - 逻辑 worker 身份
- `last_seen`
  - 最近一次 runtime touch 时间
- `is_stale`
  - 是否已经超过该 worker 自己上次上报的 `lease_timeout`
- `inflight_task_ids`
  - 当前仍归这个 worker 持有的 task
- `next_deadline`
  - 这个 worker 当前最早到期的一条 inflight lease deadline
- `lease_timeout`
  - 该 worker 上次运行时实际上使用的 lease timeout
- `heartbeat_interval`
  - 该 worker 上次运行时实际上使用的 heartbeat interval

这层的设计目的是让你运维时不用再自己回读多组 Redis key 去拼 ownership / heartbeat 状态。

## Batch 怎么用

如果你需要高吞吐 claim / resolve，不要自己在外层手写很多单条循环，直接走统一 batch API：

```rust
use halo_spider::request::Request;
use halo_spider::scheduler::{self, Scheduler, Task, TaskResolution};

let scheduler = scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:news")
    .with_worker(scheduler::Worker::new("news-worker-a"));

let claimed = scheduler.take_batch_ready(32).await?;

let leases = claimed
    .iter()
    .map(|task| task.lease.clone())
    .collect::<Vec<_>>();
scheduler.complete_batch(leases).await?;

let claimed = scheduler.take_batch_ready(8).await?;
let follow = Task::new(Request::new("https://example.com/follow"));
scheduler
    .complete_and_enqueue_batch(vec![TaskResolution::new(
        claimed[0].lease.clone(),
        vec![follow],
    )])
    .await?;
```

这层语义也要明确：

- `batch` 是统一吞吐入口，不是“多条 task 一起事务提交”
- 某一条 lease 失败时，前面已经成功的那部分不会自动回滚
- 所以排查时，还是优先看每条 lease 自己的日志和 snapshot

## 优雅退出怎么做

如果某个 worker 不是崩溃，而是准备发布、缩容或手动下线，
你通常不想等 `lease_timeout` 到期后再让别的 worker 接手。

这时可以显式调用统一的 `Scheduler::release_inflight()`：

```rust
use halo_spider::scheduler::{self, Scheduler};

let scheduler = scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:news")
    .with_worker(scheduler::Worker::new("news-worker-a"));

let released = scheduler.release_inflight().await?;
println!("released {released} inflight tasks before shutdown");

scheduler.close().await?;
```

这里的语义是：

- 只释放“当前这个 scheduler 实例对应 worker”手里的 inflight task
- 被释放的 task 会按自己的 `ready_at` 回到 `ready / delayed`
- 它适合 graceful drain，不是日常正常完成任务的主路径
- 如果 worker 是直接崩溃，还是靠 `lease_timeout` + reclaim 自动恢复
- 单纯 `enqueue()` 不会留下 worker runtime；`snapshot.workers` 更接近“真正参与过 lease 生命周期的 worker”
- 对已经注册过的 worker，空轮询 `take_ready()` 仍会续一下 `last_seen`

另外有一个实现边界：

- worker 的 `last_seen` 只会在成功的 runtime 迁移上刷新
- stale lease 失败的 `heartbeat / complete / requeue` 不会再把 worker 误记成“还活着”

## 崩溃恢复语义

- worker 正常运行时
  - engine 会按 `heartbeat_interval` 续租
- worker 崩溃或长时间失联时
  - lease timeout 到期后
  - 后续访问同 namespace 的 worker 会把 stale inflight task 回收到 `ready / delayed`
- stale lease 不允许再 `complete / requeue`
  - 旧 worker 就算晚一点恢复，也不能再用过期 lease 改写任务状态

## 10 个网站怎么分

如果你有 10 个不同网站：

- 通常还是 10 份 spider/parse 逻辑
- 每个网站最好用自己独立的 `namespace`

例如：

- `jobs:site-a`
- `jobs:site-b`
- `jobs:site-c`

这样每个站点的任务池、lease 和恢复边界都独立，不会混在一起。

## 和 checkpoint 的区别

`checkpoint` 只做静态快照保存与恢复：

- 保存当时的 `ready / delayed / inflight`
- 下次启动时恢复这份快照

它不负责：

- worker ownership
- heartbeat
- runtime reclaim

这些都属于 `scheduler::Redis` 这种 durable scheduler 的运行时语义。

## 和 store 的提交边界

这一层也要明确：

- `store` 和 `scheduler` 之间不是分布式事务
- 当前 engine 会先写 `store`，再做 `scheduler complete / complete_and_enqueue`
- 所以 `store` 成功但 `scheduler resolve` 失败时，item 仍然已经落出，这条边界按 at-least-once 理解

排查时直接看这组日志就够了：

- `engine.commit.store.ok`
  - 表示 item 已经成功写入 store
- `engine.commit.store.fail`
  - 表示 store 失败，这时日志会带 `scheduler_resolve=skipped`
- `engine.commit.scheduler_resolve.ok`
  - 表示当前 lease 的 scheduler resolve 已经完成
- `engine.commit.scheduler_resolve.fail`
  - 表示 `store` 之后的 scheduler resolve 失败，需要看当前 lease、worker、url 再做恢复判断

也就是说，这里追求的是“边界清楚、失败可诊断”，不是把 `store` 和 `scheduler` 强行做成一笔跨组件事务。
