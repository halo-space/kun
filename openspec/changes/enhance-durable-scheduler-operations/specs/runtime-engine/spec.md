# 规范增量

## ADDED Requirements

### Requirement: Redis durable scheduler exposes advanced operations without changing the shared scheduler trait

系统必须把更高阶 durable scheduler 能力继续收口在 `scheduler::Redis` 这类 durable owner 上，而不是把跨 job 运维与 Redis 专属事务语义强行塞进共享 `scheduler::Scheduler` trait。

#### Scenario: Advanced durable operations remain Redis-specific

- **WHEN** 调用方需要读取 namespace/job 级 durable scheduler 状态，或执行更高阶的运维检查
- **THEN** 它通过 `scheduler::Redis` 的 Redis-specific 能力入口完成
- **AND** 通用 `scheduler::Scheduler` trait 继续只承载共享调度语义

### Requirement: Redis durable scheduler makes lease resolution outcomes explicit

系统必须让 Redis durable scheduler 的 lease 结果边界可观测、可测试，而不是只依赖模糊字符串错误区分 ownership 冲突、stale lease 或缺失状态。

#### Scenario: Lease resolution distinguishes ownership conflict from missing inflight state

- **WHEN** 某个 worker 用错误的 `worker_id` 或过期 `lease_id` 尝试 `complete()`、`requeue()` 或 `heartbeat()`
- **THEN** durable scheduler 返回显式、稳定的结果语义，区分 ownership 冲突和 inflight 状态缺失
- **AND** 这类结果可以被 engine 和调用方稳定处理

### Requirement: Redis durable scheduler exposes namespace-level operational snapshots

系统必须让调用方能够读取 durable scheduler 的 namespace 运行时快照，至少覆盖 ready、delayed、inflight 基础计数，以及 reclaim、lease 或 ownership 相关的最小观测信息。

#### Scenario: Namespace snapshot reports current queue and lease state

- **WHEN** 调用方读取某个 Redis scheduler namespace 的运行时快照
- **THEN** 它能看到当前 `ready / delayed / inflight` 计数
- **AND** 它还能看到最小 lease/reclaim 相关状态，而不需要直接自行解析底层 Redis key

### Requirement: Redis durable scheduler supports cross-job operational inspection

系统必须为多 namespace / 多 job 的 durable scheduler 运行提供最小运维读入口，避免调用方只能靠外部约定硬编码所有 job 名称。

#### Scenario: Multiple jobs can be inspected without mixing scheduler state

- **WHEN** 调用方有多份 job/namespace 同时跑在同一个 Redis 实例中
- **THEN** 它可以按约定前缀或 registry 读取各自的 job 概览
- **AND** 不同 job 的 ready、delayed、inflight 与 reclaim 语义仍保持相互隔离
