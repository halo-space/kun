# 实施任务

## 1. 事务边界

- [x] 1.1 设计并实现 Redis durable scheduler 的显式结果语义，区分 `complete / requeue / heartbeat` 中的成功、ownership 冲突、stale lease 与缺失 inflight 状态。
  - 当前已把 Redis lease 结果收口为结构化 scheduler error，并把脚本返回码细分成 ownership 冲突、stale lease 与 inactive lease。
- [x] 1.2 评估这些结果语义如何和当前 `SpiderError` / engine task runtime 对齐，避免继续只靠字符串错误分支判断。
  - 当前 `SpiderError::Scheduler` 已承载结构化 `SchedulerError`；engine task runtime 也已按 lease-resolution error 与普通 scheduler error 分流处理。
- [x] 1.3 为事务边界补最小单元测试或契约测试，覆盖 ownership 冲突、旧 lease 与缺失状态场景。
  - 当前已补 Redis 调度测试，覆盖错误 worker、ownership 冲突、stale lease、inactive lease 与 heartbeat 的旧 lease 边界。

## 2. 观测能力

- [x] 2.1 为 `scheduler::Redis` 设计 namespace 级运行时快照结构，至少包含 `ready / delayed / inflight` 计数，以及最小 lease / reclaim 观测字段。
  - 当前已提供 `scheduler::Redis::snapshot()` 与 `scheduler::NamespaceSnapshot`，覆盖 `ready / delayed / inflight` 计数、`worker_ids`、`active_lease_count`、`deadline_count`、`reclaimed_total` 与 `reclaimed_in_refresh`。
- [x] 2.2 评估这层观测如何与现有 `stats` / `Engine::with_stats_reporter(...)` 协同，明确哪些是累计计数，哪些是即时快照。
  - 当前已明确：`scheduler::Redis::snapshot()` / `namespace_snapshots_with_prefix(...)` 读取的是 namespace 即时状态；`Engine::stats()` 与 `Engine::with_stats_reporter(...)` 仍然只承载单个 engine 生命周期内的累计计数。
- [x] 2.3 补文档和示例，说明调用方如何读取 durable scheduler 观测信息。
  - 当前已在 `docs/distributed_scheduler.md`、`README.md`、`docs/capabilities.md` 与 `examples/custom_scheduler.rs` 补 `snapshot()` 和跨 namespace 概览用法。

## 3. 跨 Job 运维

- [x] 3.1 为多 namespace / 多 job 的 Redis durable scheduler 设计最小 registry 或前缀扫描约定，避免调用方只能手写全部 job 名称。
  - 当前已为 `scheduler::Redis` 增加 Redis 内 namespace registry，并自动在运行时同步 namespace 与最小 metadata。
- [x] 3.2 提供最小 job 概览读取能力，并明确它和单 namespace scheduler 运行时语义的边界。
  - 当前已提供 `scheduler::Redis::namespaces_with_prefix(...)` 与 `scheduler::Redis::namespace_snapshots_with_prefix(...)`，并明确这层是 Redis-specific 运维读入口，不进入共享 `Scheduler` trait。
- [x] 3.3 为多 job 运维入口补测试与 `docs/distributed_scheduler.md` 说明，明确隔离、命名与使用建议。
  - 当前已补 Redis scheduler 测试覆盖 namespace registry / 批量 snapshot，并在 `docs/distributed_scheduler.md` 说明 namespace 隔离、前缀约定与读取方式。

## 4. 文档与收口

- [x] 4.1 同步 `openspec/specs/runtime-engine/spec.md`、`README.md`、`docs/capabilities.md` 与 `docs/distributed_scheduler.md`。
- [x] 4.2 完成实现后运行相关 `cargo test`，必要时补 `cargo check --examples`。
  - 当前已运行 `cargo fmt --all`、`cargo test --quiet`、`cargo check --examples --quiet`，均已通过。
