# 变更提案

## 为什么做

- 当前 `halo-spider` 的 durable scheduler 运行时后端只有 `scheduler::Redis` 一种实现。虽然 `Scheduler`、`Control`、`Worker` 这些 API 已经被收口成统一形状，但还缺一个真正不同存储模型的第二后端来验证这套抽象没有被 Redis 细节绑死。
- 用户前面已经明确提出，scheduler 是整体能力，不应该只围着 Redis 设计；后续无论接 `sqlite`、`etcd` 还是别的后端，都应该走同一套公开 API。
- 用 `sqlite` 作为第二个 durable scheduler 后端，有两个直接价值：
  - 给单机部署一个比 `Memory` 更持久、但比 Redis 更轻量的选择；
  - 用真实实现验证现在的 `Scheduler + Control + Snapshot + Worker` 抽象是否足够通用。

## 范围

- 会影响哪些 capability specs：
  - `openspec/specs/runtime-engine/spec.md`
- 会影响哪些模块 / 示例：
  - `src/scheduler/` 下新增 `sqlite` 后端
  - `src/lib.rs` 与 `src/scheduler.rs` 的公开导出
  - `examples/custom_scheduler.rs`
  - `README.md`
  - `docs/capabilities.md`
  - `docs/distributed_scheduler.md`
- 预期带来哪些用户可见结果：
  - 新增 `scheduler::Sqlite`
  - 调用方可以像 `Memory` / `Redis` 一样，通过统一 `Scheduler` 与 `Control` API 使用 SQLite durable scheduler
  - `Sqlite` 也支持 `ready / delayed / inflight`、`lease_timeout`、`heartbeat`、`snapshot()`、`overview()`、`pause_scope()`、`release_scope()` 等统一语义
  - 文档会补明确示例，说明什么时候该选 `Memory / Sqlite / Redis`

## 非目标

- 这次不做 `etcd`、ZooKeeper 这类分布式协调后端。
- 这次不把 `store -> scheduler resolve` 扩成跨组件分布式事务。
- 这次不顺带重构 browser、telemetry 或 plugin 自动装载。
- 这次不追求把 SQLite 做成跨多机共享后端；它的定位是本机 durable scheduler，而不是 Redis 替代品。

## 风险

- 是否存在兼容性或迁移风险：
  - 新增的是后端实现和导出，原则上不会破坏现有 `Memory / Redis` 用户；但如果抽象里还藏着 Redis 假设，实现过程中可能暴露出 trait 需要再微调的地方。
- 是否存在 runtime、middleware 或 plugin 相关风险：
  - SQLite 调度会引入新的持久化 runtime 路径，如果 schema、事务边界或锁策略处理不好，可能出现 claim 冲突、heartbeat 续租失败或 snapshot 视图不一致的问题。
