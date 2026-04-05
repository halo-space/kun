# 实施任务

## 1. 事务边界

- [ ] 1.1 设计并实现 Redis durable scheduler 的显式结果语义，区分 `complete / requeue / heartbeat` 中的成功、ownership 冲突、stale lease 与缺失 inflight 状态。
- [ ] 1.2 评估这些结果语义如何和当前 `SpiderError` / engine task runtime 对齐，避免继续只靠字符串错误分支判断。
- [ ] 1.3 为事务边界补最小单元测试或契约测试，覆盖 ownership 冲突、旧 lease 与缺失状态场景。

## 2. 观测能力

- [ ] 2.1 为 `scheduler::Redis` 设计 namespace 级运行时快照结构，至少包含 `ready / delayed / inflight` 计数，以及最小 lease / reclaim 观测字段。
- [ ] 2.2 评估这层观测如何与现有 `stats` / `Engine::with_stats_reporter(...)` 协同，明确哪些是累计计数，哪些是即时快照。
- [ ] 2.3 补文档和示例，说明调用方如何读取 durable scheduler 观测信息。

## 3. 跨 Job 运维

- [ ] 3.1 为多 namespace / 多 job 的 Redis durable scheduler 设计最小 registry 或前缀扫描约定，避免调用方只能手写全部 job 名称。
- [ ] 3.2 提供最小 job 概览读取能力，并明确它和单 namespace scheduler 运行时语义的边界。
- [ ] 3.3 为多 job 运维入口补测试与 `docs/distributed_scheduler.md` 说明，明确隔离、命名与使用建议。

## 4. 文档与收口

- [ ] 4.1 同步 `openspec/specs/runtime-engine/spec.md`、`README.md`、`docs/capabilities.md` 与 `docs/distributed_scheduler.md`。
- [ ] 4.2 完成实现后运行相关 `cargo test`，必要时补 `cargo check --examples`。
