# 自定义 Scheduler 后端实现指南

这份文档说明如果你想自己实现一个新的调度后端，例如 `MySQL`、`PostgreSQL`、`TiDB` 或别的存储，该怎么把它接进当前统一的 `scheduler` 抽象。

这里以 `MySQL` 为例，但重点不是某个数据库，而是统一模型本身。

## 先记住一个原则

`Memory`、`Redis`、`Sqlite`、以后用户自定义的 `MySQL`，实现方式可以不同，但它们遵守的模型约束必须一致：

- 都是同一个 `Scheduler` 抽象
- 都是同一个 `Control` 抽象
- 都复用同一个 `Worker`
- 都复用同一个 `Snapshot / Overview`
- 都必须遵守同一套 task 生命周期

也就是说，不同后端只允许内部实现不同，不允许对外模型语义不一样。

## 统一模型里必须满足的约束

一个自定义 durable scheduler 至少要保证这些事情：

### 1. 稳定 task identity

- 每条调度任务都必须有稳定 `task_id`
- ack / requeue / heartbeat / reclaim 都要围绕 `task_id` 运转
- 不能只拿 URL 当唯一标识

### 2. 三段状态一致

所有后端都必须维护这三种状态：

- `ready`
- `delayed`
- `inflight`

并且状态迁移要和内置后端一致：

- `enqueue` 把任务放进 `ready` 或 `delayed`
- `take_ready` 把任务从 `ready` 领到 `inflight`
- `complete` 把 `inflight` 删除
- `requeue` 把 `inflight` 放回 `ready / delayed`
- `complete_and_enqueue` 是“当前 lease 完成 + follow task 入队”的单后端事务边界

### 3. 排序语义一致

当前统一排序约束是：

- `priority` 高的先取
- `priority` 相同，`depth` 小的先取
- 再相同，用后端内部的稳定顺序字段打平

对 `MySQL` 来说，通常要有一个每个 scope 内单调递增的 `sequence` 字段。

### 4. delayed 语义一致

- `ready_at <= now` 的 delayed task 要能晋升回 `ready`
- `requeue` 时也要重新判断它应该回到 `ready` 还是 `delayed`

### 5. worker / lease 语义一致

一个 durable 后端必须实现：

- `worker_id`
- `lease_id`
- `lease_timeout`
- `heartbeat`
- stale inflight reclaim

并且要区分这几种失败：

- `LeaseWorkerMismatch`
- `LeaseOwnershipConflict`
- `StaleLease`
- `InactiveLease`

### 6. 观测形状一致

自定义后端不只是能调度，还要能读统一运行态：

- `checkpoint()`
- `counts()`
- `snapshot()`
- `scopes()`
- `snapshots()`
- `overview()`

如果你的后端是共享后端，例如一个 MySQL 库里管理多个 job/scope，就应该真正实现跨 scope 读取，而不是只返回当前 scope。

### 7. 运维控制一致

如果是 durable/shared backend，建议直接实现完整 `Control`：

- `pause_scope()`
- `resume_scope()`
- `release_scope()`
- `purge_scope()`

## 推荐的 MySQL 表结构

最自然的方式是“单库多 scope”。

### `scheduler_scopes`

用于保存 scope 元信息：

```sql
CREATE TABLE scheduler_scopes (
    scope VARCHAR(255) PRIMARY KEY,
    is_paused BOOLEAN NOT NULL DEFAULT FALSE,
    reclaimed_total BIGINT NOT NULL DEFAULT 0,
    next_sequence BIGINT NOT NULL DEFAULT 0,
    lease_timeout_ms BIGINT NULL,
    heartbeat_interval_ms BIGINT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);
```

### `scheduler_tasks`

用于保存任务状态：

```sql
CREATE TABLE scheduler_tasks (
    scope VARCHAR(255) NOT NULL,
    task_id VARCHAR(255) NOT NULL,
    task_json JSON NOT NULL,
    state VARCHAR(16) NOT NULL,
    priority INT NOT NULL,
    depth INT NOT NULL,
    ready_at_ms BIGINT NULL,
    sequence BIGINT NOT NULL,
    worker_id VARCHAR(255) NULL,
    lease_id VARCHAR(255) NULL,
    deadline_ms BIGINT NULL,
    claimed_at_ms BIGINT NULL,
    PRIMARY KEY (scope, task_id),
    INDEX idx_tasks_ready (scope, state, priority DESC, depth ASC, sequence ASC),
    INDEX idx_tasks_delayed (scope, state, ready_at_ms ASC, sequence ASC),
    INDEX idx_tasks_inflight (scope, state, deadline_ms ASC, worker_id, lease_id)
);
```

### `scheduler_workers`

用于保存 worker runtime 信息：

```sql
CREATE TABLE scheduler_workers (
    scope VARCHAR(255) NOT NULL,
    worker_id VARCHAR(255) NOT NULL,
    last_seen_ms BIGINT NOT NULL,
    lease_timeout_ms BIGINT NULL,
    heartbeat_interval_ms BIGINT NULL,
    PRIMARY KEY (scope, worker_id)
);
```

## 推荐事务边界

这里说的事务边界，是 scheduler 自己内部的单后端事务，不是跨 `store / scheduler` 的分布式事务。

### `enqueue`

一个事务里完成：

- 确保 scope 元信息存在
- 分配 `sequence`
- 插入 task

### `take_ready`

一个事务里完成：

- reclaim 过期 inflight
- promote 已到期 delayed
- 读取 `is_paused`
- 按统一排序挑一条 ready
- 把它更新成 inflight
- 写入 `worker_id / lease_id / deadline`
- 更新 worker runtime

如果你用的是 MySQL 8，推荐优先考虑：

- `SELECT ... FOR UPDATE SKIP LOCKED`

这样多 worker 并发 claim 更自然。

### `complete`

一个事务里完成：

- 校验当前 `task_id / worker_id / lease_id`
- 删除 inflight task
- 如果当前 worker 已没有 inflight，可以顺手清理 worker runtime

### `requeue`

一个事务里完成：

- 校验 lease
- 重新判断回到 `ready` 还是 `delayed`
- 清掉 `worker_id / lease_id / deadline`
- 分配新的 `sequence`

### `heartbeat`

一个事务里完成：

- 校验 lease
- 续租 `deadline_ms`
- 刷新 worker `last_seen_ms`

### `complete_and_enqueue`

这是 durable backend 里很重要的一个单后端事务：

- follow task 入队
- 当前 lease 完成

这两步最好在同一个数据库事务里完成。

### `release_scope`

一个事务里完成：

- 找出这个 scope 里所有 inflight
- 按 task 自身 `ready_at` 判断回到 `ready / delayed`
- 清理 worker runtime

### `purge_scope`

一个事务里完成：

- 删除 scope 下所有任务
- 删除 worker runtime
- 重置 scope 元信息

## Rust 侧实现骨架

真正的对接边界就是：

- `scheduler::Scheduler`
- `scheduler::Control`

下面是一个最小骨架：

```rust
use halo_spider::error::SpiderError;
use halo_spider::scheduler::checkpoint::{Checkpoint, Counts};
use halo_spider::scheduler::{ClaimedTask, Control, Scheduler, Snapshot, Task, TaskLease, Worker};

pub struct MysqlScheduler {
    url: String,
    scope: String,
    worker: Worker,
}

impl MysqlScheduler {
    pub fn new(url: impl Into<String>, scope: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            scope: scope.into(),
            worker: Worker::new("mysql-worker"),
        }
    }

    pub fn with_worker(mut self, worker: Worker) -> Self {
        self.worker = worker;
        self
    }
}

impl Scheduler for MysqlScheduler {
    async fn enqueue(&self, task: Task) -> Result<(), SpiderError> {
        todo!("在 MySQL 事务里入队")
    }

    async fn checkpoint(&self) -> Result<Checkpoint, SpiderError> {
        todo!("导出当前 scope 的 ready/delayed/inflight")
    }

    async fn counts(&self) -> Result<Counts, SpiderError> {
        todo!("统计当前 scope 的 ready/delayed/inflight")
    }

    async fn snapshot(&self) -> Result<Snapshot, SpiderError> {
        todo!("读取统一 Snapshot 形状")
    }

    async fn take_ready(&self) -> Result<Option<ClaimedTask>, SpiderError> {
        todo!("claim ready -> inflight")
    }

    async fn complete(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        todo!("校验 lease 后完成")
    }

    async fn requeue(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        todo!("校验 lease 后退回 ready/delayed")
    }

    async fn release_inflight(&self) -> Result<usize, SpiderError> {
        todo!("把当前 worker 已 claim 的 inflight 主动交回 scheduler")
    }

    async fn heartbeat(&self, lease: &TaskLease) -> Result<(), SpiderError> {
        todo!("续租 deadline + 刷新 worker last_seen")
    }

    async fn has_pending(&self) -> Result<bool, SpiderError> {
        Ok(self.counts().await?.has_pending())
    }
}

impl Control for MysqlScheduler {
    async fn pause_scope(&self, scope: &str) -> Result<bool, SpiderError> {
        todo!("设置 scope paused")
    }

    async fn resume_scope(&self, scope: &str) -> Result<bool, SpiderError> {
        todo!("恢复 scope claim")
    }

    async fn release_scope(&self, scope: &str) -> Result<usize, SpiderError> {
        todo!("回收整个 scope 的 inflight")
    }

    async fn purge_scope(&self, scope: &str) -> Result<Counts, SpiderError> {
        todo!("清空整个 scope")
    }
}
```

仓库里也有一份对应骨架文件：

- `examples/custom_scheduler_mysql.rs`

## 建议的实现顺序

不要一上来就把所有运维能力一起写完，最稳的顺序是：

1. 先把 `enqueue / take_ready / complete / requeue` 跑通
2. 再补 `heartbeat / reclaim / release_inflight`
3. 再补 `snapshot / scopes / snapshots / overview`
4. 最后补 `Control`

但注意，这只是实现顺序，不代表模型可以长期缺一半。最终对外能力面仍然应该和内置后端一致。

## 怎么挂到 Engine

当你的后端实现完 `Scheduler`，就可以直接接入：

```rust
use halo_spider::engine::Engine;
use halo_spider::scheduler::Worker;

let scheduler = MysqlScheduler::new(
    "mysql://user:pass@127.0.0.1:3306/kun",
    "jobs:news",
)
.with_worker(
    Worker::new("news-worker-a")
        .with_lease_timeout(jiff::SignedDuration::from_secs(30))
        .with_heartbeat_interval(jiff::SignedDuration::from_secs(10)),
);

let engine = Engine::new().with_scheduler(scheduler);
```

## 建议的契约测试

自定义 MySQL backend 最少要补这些测试：

- `enqueue -> take_ready -> complete`
- priority / depth 排序
- delayed task promotion
- `complete_and_enqueue`
- `complete_batch / requeue_batch`
- foreign worker 不能完成别人的 lease
- stale lease 不能继续 resolve
- heartbeat 能续租
- lease timeout 后能 reclaim
- `snapshot / scopes / snapshots / overview`
- `pause_scope / resume_scope / release_scope / purge_scope`

如果这些都过了，你的自定义后端通常就已经真正接上当前统一调度模型了。
