# 技术设计

## 概览

- 这次变更不重写当前 scheduler 架构，而是在现有 `scheduler::Redis` durable 语义之上继续补第二阶段能力。
- 设计重点分成三组：
  - 事务边界：把当前“Redis 脚本原子迁移”进一步收口成更明确的运行时结果语义，区分成功、ownership 冲突、stale lease、缺失任务等边界。
  - 观测：为 durable scheduler 补 namespace/job 级可读状态，而不是只让调用方从底层 Redis key 推断运行状态。
  - 跨 job 运维：在不污染通用 `scheduler::Scheduler` trait 的前提下，为 Redis durable scheduler 增加最小 job/namespace 管理入口。

## 模块影响

- `src/scheduler/redis.rs`
  - 继续作为 durable scheduler owner，补事务结果、运行时快照与 job 级运维入口。
- `src/engine.rs`
  - 如果需要把 scheduler 观测接到 engine 生命周期，需要补最小接线，但不改变现有 `Engine::new()` 默认组合。
- `src/engine/task.rs`
  - 如果 lease heartbeat、ownership 冲突或 reclaim 要产生更明确的运行时结果，需要在任务执行路径里收口处理。
- `src/stats.rs`
  - 如果 durable scheduler 观测继续挂到统一 stats/reporter 链路，需要补最小计数或事件类型。
- `examples/custom_scheduler.rs`
  - 补最小可运行示例，展示 durable scheduler 第二阶段能力如何读取 job 状态或冲突结果。
- `docs/distributed_scheduler.md`
  - 补多 job 运维与观测说明，明确这层能力属于 Redis durable scheduler，而不是通用 checkpoint。
- `README.md`
  - 把 durable scheduler 的“当前已完成核心语义”与“后续增强项”继续明确分开。
- `openspec/specs/runtime-engine/spec.md`
  - 增加 durable scheduler 第二阶段能力的规范增量。

## 关键决策

- Runtime / middleware 影响：
  - 这次不新增新的 middleware owner。
  - durable scheduler 的更高阶事务与运维语义继续属于 `scheduler::Redis` 和 engine task runtime 的职责，不下沉到 middleware。
- 对外 API 影响：
  - 不把更高阶运维能力强塞进通用 `scheduler::Scheduler` trait。
  - 通用 trait 仍只承担 enqueue / take_ready / complete / requeue / heartbeat / checkpoint 这组共享调度语义。
  - Redis durable scheduler 的高级能力优先通过 Redis-specific API 暴露，例如 job/namespace 快照、冲突结果与运维读接口。
- Plugin 或 DSL 影响：
  - 这次不扩 plugin kind。
  - 这次不扩 DSL/rules 调度配置面，避免在共享底层能力稳定前把分布式运维语义提前暴露到 DSL。

## 验证方式

- 先补 `openspec/changes/enhance-durable-scheduler-operations/specs/runtime-engine/spec.md`，明确事务边界、观测与跨 job 运维的规范。
- 再补 Redis durable scheduler 的单元测试或契约测试，至少覆盖：
  - ownership/stale lease 的显式结果边界
  - namespace/job 级状态读取
  - 多 namespace 并存时的 registry/运维语义
- 最后同步 `README.md`、`docs/capabilities.md` 与 `docs/distributed_scheduler.md`，确保对外表述和实现一致。
