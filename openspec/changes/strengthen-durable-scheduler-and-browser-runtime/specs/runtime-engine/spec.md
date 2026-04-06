# 规范增量

## ADDED Requirements

### Requirement: Durable scheduler snapshot exposes inflight ownership details

系统 MUST 让 `scheduler::Redis` 的运行时 snapshot 不仅返回 namespace 级聚合计数，也能返回当前 inflight task 的 ownership / lease / deadline 明细。

#### Scenario: Redis scheduler snapshot shows inflight lease ownership

- **WHEN** 某个 Redis durable scheduler namespace 当前存在 inflight task
- **THEN** `scheduler::Redis::snapshot()` 返回的结构里包含这条 inflight task 的 task identity、worker identity、lease identity 与当前 deadline
- **AND** 调用方不需要手工回读底层 Redis key 才能定位 ownership 状态

#### Scenario: Namespace snapshots keep per-task ownership visibility across namespaces

- **WHEN** 调用方读取 `scheduler::Redis::namespace_snapshots(...)` 或 `namespace_snapshots_with_prefix(...)`
- **THEN** 每个 namespace snapshot 都继续包含对应 inflight task 的 ownership / lease 明细
- **AND** 这些明细与 namespace 级 ready / delayed / inflight 聚合计数保持一致

### Requirement: Browser request supports structured custom fingerprint profiles

系统 MUST 允许 browser request 除了使用内置 `fingerprint_profile` 名称外，还能声明结构化自定义 fingerprint profile。

#### Scenario: Browser request can provide a custom structured profile

- **WHEN** 调用方对 browser request 显式提供结构化 fingerprint profile
- **THEN** browser downloader 使用这份 profile 生成执行计划
- **AND** 调用方不需要强行把自定义 profile 挂成新的内置 preset 名称

#### Scenario: Builtin profile names remain available

- **WHEN** 调用方继续使用内置 `fingerprint_profile` 名称
- **THEN** browser downloader 继续解析这些稳定 preset
- **AND** 新的结构化 profile 能力不会破坏现有 preset 路径

### Requirement: Browser session reuse policy is explicit

系统 MUST 让 browser `session` 的 live reuse 策略成为显式配置，而不是只隐含在稳定 user data dir 复用里。

#### Scenario: Browser request can choose a session reuse policy

- **WHEN** 调用方对 browser request 显式声明 session reuse 策略
- **THEN** browser runtime 按该策略决定是否复用 live context 或 page
- **AND** 旧的仅 user data dir 复用路径仍然可以继续保留

#### Scenario: Session reuse stays scoped to one logical session

- **WHEN** 两个 browser request 使用不同的 session id
- **THEN** 它们不会意外共享同一个 live context 或 live page
- **AND** reuse 仍然受 session identity 边界约束

## MODIFIED Requirements

### Requirement: Scheduler Uses Stable Task Identity

系统 MUST 使用稳定的 task identity 跟踪 ready、delayed 与 inflight 任务，并让 durable scheduler 的运维视图能直接反映 inflight ownership。

#### Scenario: Redis durable scheduler snapshot remains an operational view

- **WHEN** 调用方通过 `scheduler::Redis::snapshot()` 读取 namespace 即时状态
- **THEN** 它除了 ready / delayed / inflight 聚合计数外，还能看到当前 inflight ownership 明细
- **AND** 这份视图继续表示 durable scheduler 的当前运行态，而不是 checkpoint 快照

## REMOVED Requirements
