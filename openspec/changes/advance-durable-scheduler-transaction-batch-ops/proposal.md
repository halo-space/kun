# 变更提案

## 为什么做

- `halo-spider` 当前的 durable scheduler 已经可用：`scheduler::Redis` 已经覆盖 `ownership`、`heartbeat`、`stale reclaim`、`snapshot / overview` 与最小 runtime 观测。
- 但现在这层还停留在“单条任务状态迁移是原子的”，引擎级提交边界还不够清晰。`store` 写入、follow/retry 回队与 scheduler resolve 之间一旦进程中断，调用方仍然需要自己理解重复消费或重复写入边界。
- 当前共享 `Scheduler` trait 也还是单条任务接口为主，吞吐提升主要依赖并发，而没有统一的 batch 调度接口。
- 此外，虽然现在已经能读 `snapshot()`、`overview()` 和跨 scope 视图，但更明确的跨 job 运维控制面还没有收口，调用方还缺少稳定的“控制动作 + 示例”入口。

## 范围

- 会影响哪些 capability specs：
  - `openspec/specs/runtime-engine/spec.md`
- 会影响哪些模块 / 示例：
  - `src/scheduler/traits.rs`
  - `src/scheduler/memory.rs`
  - `src/scheduler/redis.rs`
  - `src/scheduler/runtime.rs`
  - `src/engine/task.rs`
  - `src/engine.rs`
  - `README.md`
  - `docs/capabilities.md`
  - `docs/distributed_scheduler.md`
  - `examples/custom_scheduler.rs`
- 预期带来哪些用户可见结果：
  - engine 对 scheduler resolve、follow/retry enqueue、store write 的事务边界更明确
  - `Scheduler` 增加统一 batch API，并继续适用于 `Memory / Redis / 以后其它后端`
  - 跨 job 运维控制面形成清晰的统一入口，并补具体使用示例

## 非目标

- 这次 change 不实现独立 maintenance/operator 进程，也不把自动巡检、告警、后台修复全部一次做完。
- 这次 change 不引入新的分布式协调系统，不实现 leader election、跨实例事务或全局一致性协议。
- 这次 change 不实现第二个 durable backend；`sqlite` durable backend 放到下一阶段单独验证抽象，不混进这一轮。
- 这次 change 不改变 `checkpoint` 只是静态快照恢复边界这一原则。

## 风险

- 是否存在兼容性或迁移风险：
  - 有。共享 `Scheduler` trait 一旦增加 batch API，需要保证现有自定义 scheduler 仍然可以通过默认实现平滑过渡。
- 是否存在 runtime、middleware 或 plugin 相关风险：
  - 有。引擎级事务边界如果设计不清楚，很容易让 `store`、follow/retry 与 scheduler resolve 再次出现重复提交或语义模糊。
