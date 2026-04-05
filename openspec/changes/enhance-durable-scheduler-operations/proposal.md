# 变更提案

## 为什么做

- `halo-spider` 当前已经有可用的 durable scheduler：`scheduler::Redis` 已经覆盖最小 worker ownership、lease heartbeat、stale inflight reclaim 与原子状态迁移。
- 但这一层现在仍然主要解决“多 worker 不重复领任务、worker 崩溃后任务能回收”这类核心运行时问题；对于更长期的分布式运维，还缺少更明确的事务边界、观测语义与跨 job 运维能力。
- 如果不把这组增强单独收口成正式 change，README、capabilities 和后续实现里很容易继续把“已经完成的最小 durable 语义”和“下一阶段的高阶分布式能力”混在一起，影响对外表达和后续任务拆分。

## 范围

- 会影响哪些 capability specs：
  - `openspec/specs/runtime-engine/spec.md`
- 会影响哪些模块 / 示例：
  - `src/scheduler/redis.rs`
  - `src/engine.rs`
  - `src/engine/task.rs`
  - `src/stats.rs`
  - `docs/distributed_scheduler.md`
  - `docs/capabilities.md`
  - `README.md`
  - `examples/custom_scheduler.rs`
- 预期带来哪些用户可见结果：
  - 明确 durable scheduler 当前的事务边界，不再让跨 worker 的状态收口只停留在最小脚本原子性层面
  - 为 `scheduler::Redis` 补一层更稳定的观测语义，至少能让调用方看到 lease、reclaim、ownership 冲突与 job 级状态
  - 为多 job / 多 namespace 的运行补一层更明确的运维入口，而不是只提供单 namespace 的最低层调度接口

## 非目标

- 这次 change 不重做 `scheduler::Memory` 的核心语义。
- 这次 change 不扩展 DSL / rules 层的调度配置面。
- 这次 change 不引入新的通用分布式控制平面，也不承诺“一次做完”完整集群调度系统。
- 这次 change 不改变 `checkpoint` 只负责静态快照恢复的职责边界。

## 风险

- 是否存在兼容性或迁移风险：
  - 存在。durable scheduler 如果补更严格的事务边界或 ownership 校验，可能会让当前较宽松的 Redis 访问路径变成显式失败。
- 是否存在 runtime、middleware 或 plugin 相关风险：
  - 存在。调度观测和跨 job 运维如果没有和现有 `stats`、`Engine`、`scheduler` 边界一起收口，容易再次引入新的 owner 混乱或并行的管理入口。
