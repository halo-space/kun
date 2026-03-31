# 规范增量

## ADDED Requirements

### Requirement: Scheduler Uses Stable Task Identity

系统 MUST 使用稳定的 task identity 跟踪 ready、delayed 与 inflight 任务，而不是仅用 URL 作为 ack/nack 标识。

#### Scenario: Same URL with different request context can coexist

- **WHEN** 两个请求 URL 相同，但 method、body 或 meta 不同
- **THEN** scheduler 能够正确区分并独立 ack/nack 它们

#### Scenario: Retry keeps the same task identity

- **WHEN** 某个 inflight task 因失败被重试或延迟重排
- **THEN** 该任务在 ready、delayed 与 inflight 之间流转时保持同一个 task identity，而不是重新退化成按 URL 跟踪

#### Scenario: Memory scheduler keeps its scheduler state boundary explicit

- **WHEN** 当前使用 `scheduler::Memory`
- **THEN** 它把 scheduler state 明确为 `ready`、`delayed` 与 `inflight` 三组任务状态
- **AND** 这些状态可以导出为共享 `scheduler::state::Snapshot`，而不是只藏在内存实现细节里

#### Scenario: Durable scheduler state is a separate persistence concern

- **WHEN** 调用方需要 crash-safe scheduler state/scheduler
- **THEN** 它应基于共享 `scheduler::state::Snapshot` / `scheduler::state::Store` 边界落到独立持久化实现
- **AND** 当前 `scheduler::Memory` 的边界保持为 memory-only

### Requirement: Engine Defines Validation And Item Pipeline Failure Semantics

系统 MUST 明确 validation、单一 item pipeline 以及 item 丢弃时的引擎行为语义。

#### Scenario: Validation failure is handled explicitly

- **WHEN** 解析结果未通过共享 validation
- **THEN** 引擎依据明确规则决定报错、丢弃或其他可配置行为

#### Scenario: Pipeline failure does not rely on implicit best effort

- **WHEN** pipeline 处理 item 失败
- **THEN** 引擎遵循明确、可测试的错误处理策略

#### Scenario: Pipeline can drop items explicitly

- **WHEN** pipeline 对某个 item 返回 `Ok(false)`
- **THEN** 引擎显式丢弃该 item，而不是再依赖独立 sink 决定最终保留语义

### Requirement: HTTP Downloader Wires Shared Transport Semantics

系统 MUST 在 HTTP downloader 中把 timeout、cookie jar、proxy 与 redirect 统一接到共享 request 语义上。

#### Scenario: Session requests share cookie jar state

- **WHEN** 多个请求复用同一个 session 标识
- **THEN** 前一跳响应写入的 cookie 会进入同一 cookie jar，并作用到后续请求

#### Scenario: Request timeout and proxy settings are executed by downloader

- **WHEN** HTTP request 显式声明 timeout、proxy 或 redirect 行为
- **THEN** downloader 依据这些 request 语义执行真实网络行为，而不是忽略或分散到不一致实现中
