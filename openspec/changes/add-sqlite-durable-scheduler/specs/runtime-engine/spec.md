# 规范增量

## ADDED Requirements

### Requirement: SQLite durable scheduler backend is available

系统 MUST 提供内置 `scheduler::Sqlite`，作为共享 `Scheduler` 抽象下的 durable scheduler 后端，而不是让持久化调度只停留在 Redis 一种实现。

#### Scenario: Callers can use SQLite as a durable scheduler backend

- **WHEN** 调用方使用内置 `scheduler::Sqlite`
- **THEN** 它直接以 SQLite 持久化 `ready`、`delayed` 与 `inflight` 任务状态
- **AND** 它继续遵守共享的 task identity、priority、depth 与 requeue 语义

#### Scenario: SQLite scheduler supports the same worker runtime semantics

- **WHEN** 调用方给 `scheduler::Sqlite` 配置 `worker_id`、`lease_timeout` 与 `heartbeat_interval`
- **THEN** 它也支持 worker ownership 校验、heartbeat 续租与 stale inflight reclaim
- **AND** engine 不需要为 SQLite 单独增加另一套调度接线

#### Scenario: SQLite scheduler participates in the same read and control APIs

- **WHEN** 调用方使用 `scheduler::Sqlite`
- **THEN** 它也通过共享 `Scheduler` 提供 `checkpoint()`、`counts()`、`snapshot()`、`scopes()`、`snapshots()`、`overview()`
- **AND** 通过共享 `Control` 提供 `pause_scope()`、`resume_scope()`、`release_scope()`、`purge_scope()`

## MODIFIED Requirements

### Requirement: Scheduler Uses Stable Task Identity

系统 MUST 使用稳定的 task identity 跟踪 ready、delayed 与 inflight 任务，而不是仅用 URL 作为 ack/nack 标识。

#### Scenario: Custom scheduler and checkpoint backends remain extensible

- **WHEN** 调用方需要自定义 scheduler 或 checkpoint 持久化后端
- **THEN** 它可以分别实现 `scheduler::Scheduler` 或 `scheduler::checkpoint::Persist`
- **AND** 当前内置后端至少包含 `Memory`、`Redis` 与 `Sqlite`
- **AND** 它们继续复用共享的 task state、worker 与 control 边界
