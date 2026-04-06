# Distributed Scheduler

`scheduler::Redis` 是运行时调度器，不是 checkpoint。

只要某个 `scheduler::Redis` 真正参与过 enqueue / claim / snapshot / counts 这类访问，
它的 namespace 就会自动登记到同一个 Redis 里的 durable scheduler registry。

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
    scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:news")
        .with_worker_id("news-worker-a")
        .with_lease_timeout(jiff::SignedDuration::from_secs(30))
        .with_heartbeat_interval(jiff::SignedDuration::from_secs(10)),
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
    .with_worker_id("news-worker-a");

let worker_b = scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:news")
    .with_worker_id("news-worker-b");
```

这样两个 worker 会从同一个任务池里 claim task，但不会重复 claim 同一条 ready task。

## 读取 namespace 运行时快照

如果你想直接看某个 namespace 当前的 durable scheduler 状态，可以调用
`scheduler::Redis::snapshot()`：

```rust
use halo_spider::scheduler;

let scheduler = scheduler::Redis::new("redis://127.0.0.1:6379", "jobs:news")
    .with_worker_id("ops-reader");

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
- `snapshot()` / `counts()` / `checkpoint()` 不会把当前调用方登记成活跃 worker；真正刷新 worker runtime 的只有 `enqueue / take_ready / complete / requeue / heartbeat`

## 跨 job 运维怎么读

如果同一个 Redis 里跑了多个 namespace，可以先按前缀发现它们，再批量读取各自概览：

```rust
use halo_spider::scheduler;

let namespaces =
    scheduler::Redis::namespaces_with_prefix("redis://127.0.0.1:6379", "jobs:").await?;
println!("registered namespaces: {:?}", namespaces);

let snapshots =
    scheduler::Redis::namespace_snapshots_with_prefix("redis://127.0.0.1:6379", "jobs:").await?;

for snapshot in snapshots {
    println!("namespace: {}", snapshot.namespace);
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

- `namespaces_with_prefix(...)` / `namespace_snapshots_with_prefix(...)` 是 Redis durable scheduler 的运维读入口
- 它们不会改变共享 `scheduler::Scheduler` trait
- `namespace_snapshots_with_prefix(...)` 读取时会顺带刷新各 namespace 的 stale reclaim，所以它看到的是“这次读取后”的即时状态

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
