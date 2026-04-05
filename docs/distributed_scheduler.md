# Distributed Scheduler

`scheduler::Redis` 是运行时调度器，不是 checkpoint。

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
