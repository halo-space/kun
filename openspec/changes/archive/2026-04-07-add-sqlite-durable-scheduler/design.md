# 技术设计

## 概览

- 新增一个内置 `scheduler::Sqlite`，作为 `Memory` 与 `Redis` 之外的第二个 durable scheduler 后端。
- `Sqlite` 继续复用现有共享抽象：
  - 任务执行与状态流转走 `scheduler::Scheduler`
  - 跨 scope 运维控制走 `scheduler::Control`
  - worker 身份与 lease/heartbeat 配置继续走 `scheduler::Worker`
  - 运行态观测继续复用 `Snapshot / Overview / RuntimeEvent`
- 实现目标不是把 SQLite 做成“分布式 Redis 替代品”，而是提供一个单机 durable scheduler，并用真实后端验证当前抽象没有被 Redis 特化语义绑死。

## 模块影响

- `src/scheduler/sqlite.rs`
  - 新增 SQLite durable scheduler 实现
  - 负责 `ready / delayed / inflight / worker runtime / scope meta` 的存储与迁移
- `src/scheduler.rs`
  - 导出 `scheduler::Sqlite`
- `src/lib.rs`
  - 如果当前导出链需要显式补模块，这里同步导出
- `src/scheduler/runtime.rs`
  - 继续复用，不新增第二套事件模型
- `src/scheduler/snapshot.rs`
  - 直接复用现有 `Snapshot / Overview` 结构，不为 SQLite 再造特化视图
- `examples/custom_scheduler.rs`
  - 增加 `scheduler::Sqlite` 用法示例
- `README.md`
  - 补 “什么时候选 `Memory / Sqlite / Redis`”
- `docs/capabilities.md`
  - 补 SQLite durable scheduler 能力边界
- `docs/distributed_scheduler.md`
  - 说明 SQLite 的定位是单机 durable，而不是跨多机共享
- `openspec/specs/runtime-engine/spec.md`
  - 补 SQLite durable scheduler requirement/scenario

## 关键决策

- Runtime / middleware 影响：
  - 不新增新的 middleware 或 engine 分支。
  - engine 仍然只依赖共享 `Scheduler / Control / Worker` 语义；SQLite 后端自己负责 claim、complete、requeue、heartbeat 等状态迁移。
- 对外 API 影响：
  - 新增 `scheduler::Sqlite::new(path, scope)` 这类构造入口。
  - 用户侧 API 命名与 `Memory / Redis` 保持一致：`with_worker(...)`、`snapshot()`、`overview()`、`pause_scope()` 等都不改名。
  - SQLite 后端的 scope 语义继续沿用统一 scheduler scope，而不是单独引入别的命名。
- Plugin 或 DSL 影响：
  - 这次不改 plugin 自动装载。
  - DSL 不需要感知 SQLite 特殊语义；它只继续依赖共享 scheduler 抽象。
- 存储策略：
  - 先采用单库多 scope 的 schema，而不是每个 scope 一个独立文件。
  - 事务边界先收在单 backend 内：claim / complete / requeue / heartbeat / release_scope / purge_scope 都在 SQLite 事务中完成。
  - batch API 继续只是吞吐优化，不承诺跨 task 整体事务。
- 抽象验证策略：
  - 如果实现 SQLite 过程中暴露出 trait 里仍有 Redis 特定假设，应优先把公共抽象修正为真正 backend-agnostic，再补具体实现。

## 验证方式

- 为 `scheduler::Sqlite` 增加契约测试，覆盖：
  - enqueue / take_ready / complete
  - delayed task promotion
  - stale inflight reclaim
  - worker ownership / stale lease 校验
  - heartbeat 续租
  - `snapshot()` / `snapshots_with_prefix()` / `overview_with_prefix()`
  - `pause_scope()` / `resume_scope()` / `release_scope()` / `purge_scope()`
  - batch API
- 跑 `cargo check`
- 跑 `cargo check --examples`
- 跑 SQLite 相关 targeted tests
