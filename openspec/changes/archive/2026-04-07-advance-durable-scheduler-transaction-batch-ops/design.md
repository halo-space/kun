# 技术设计

## 概览

- 这次变更只聚焦 durable scheduler 的下一阶段主线，不再混 browser 或其它能力：
  - 事务边界：明确 engine 在任务成功、失败、重试时，`store`、follow/retry 与 scheduler resolve 的提交顺序和结果语义
  - batch 调度：为共享 `Scheduler` trait 增加统一 batch API，提升吞吐，同时保持现有单条 API 可用
  - 跨 job 运维控制：在统一抽象上补可读、可操作的控制入口和示例

## 模块影响

- `src/scheduler/traits.rs`
  - 新增统一 batch 调度接口，命名统一使用 `batch`
  - 保留现有单条接口，默认实现可回退到单条路径
- `src/scheduler/memory.rs`
  - 实现 batch API，保证本地调度和共享后端的形状一致
- `src/scheduler/redis.rs`
  - 实现 batch API
  - 在现有跨 scope 读能力之上，补统一的运维控制入口
- `src/scheduler/runtime.rs`
  - 让 runtime event / metrics 对 batch 语义保持一致，不引入第二套事件体系
- `src/engine/task.rs`
  - 明确任务成功、失败、重试时的事务边界与提交顺序
- `src/engine.rs`
  - 如果需要把 batch 调度接到主循环，这里补最小接线
- `docs/distributed_scheduler.md`
  - 补跨 job 运维控制的具体操作说明
- `README.md` / `docs/capabilities.md`
  - 同步事务边界、batch API 与运维入口说明

## 关键决策

- Runtime / middleware 影响：
  - 不引入新的 middleware owner
  - 事务边界继续由 `engine + scheduler + store` 共同收口，不拆成新的 runtime 子系统
- 对外 API 影响：
  - 共享 batch 接口统一使用 `batch` 命名，不使用 `many`
  - `release_scope` 这类动作不属于 batch，而属于运维控制入口
  - 运维控制优先做统一入口，不让用户直接面向 Redis 专属命名来理解整套能力
- 事务边界策略：
  - 这次先把“顺序与结果语义”收清楚，不承诺跨外部 store 和 scheduler 的真正分布式事务
  - 对外文档要明确哪些场景是 at-least-once，哪些场景已经做到单 backend 原子迁移
- 后续抽象验证：
  - 第二个 durable backend 后面改用 `sqlite` 验证，不使用 `etcd`
  - 但 `sqlite` 不进入这次 change 范围

## API 方向

- 共享 batch API 候选：
  - `take_batch_ready(limit)`
  - `complete_batch(leases)`
  - `requeue_batch(leases)`
  - `complete_and_enqueue_batch(entries)`
- 跨 job 运维控制候选：
  - `scopes()` / `scopes_with_prefix(...)` 继续保留为读入口
  - 在此基础上补稳定的控制动作，例如 `pause_scope(...)`、`resume_scope(...)`、`release_worker(...)`、`purge_scope(...)`
  - 具体命名和抽象层级在实现前再一起确认，但不再把这些动作混进 batch API

## 验证方式

- 为 batch API 增加 `Memory` 与 `Redis` 双实现测试，覆盖：
  - batch claim 顺序
  - batch complete / requeue / complete_and_enqueue 结果一致性
- 为引擎级事务边界补测试，覆盖：
  - `store` 成功后再 resolve scheduler
  - `store` 失败时不错误提交 follow/retry
  - lease 丢失时的显式结果边界
- 为跨 job 运维控制补示例和文档，保证调用方有真实操作路径
