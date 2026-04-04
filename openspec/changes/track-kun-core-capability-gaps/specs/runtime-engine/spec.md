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
- **AND** 这些状态可以导出为共享 `scheduler::checkpoint::Checkpoint`，而不是只藏在内存实现细节里

#### Scenario: Durable scheduler state is a separate persistence concern

- **WHEN** 调用方需要 crash-safe scheduler state/scheduler
- **THEN** 它应基于共享 `scheduler::checkpoint::Checkpoint` / `scheduler::checkpoint::Persist` 边界落到独立持久化实现
- **AND** 当前 `scheduler::Memory` 的边界保持为 memory-only

#### Scenario: File durable scheduler checkpoint persistence is available

- **WHEN** 调用方使用内置 `scheduler::checkpoint::File`
- **THEN** 它可以把共享 `scheduler::checkpoint::Checkpoint` 保存到文件
- **AND** `scheduler::checkpoint::Memory` 可以从同一个文件 checkpoint 持久化实现恢复任务状态

#### Scenario: Redis durable scheduler checkpoint persistence is available

- **WHEN** 调用方使用内置 `scheduler::checkpoint::Redis`
- **THEN** 它可以把共享 `scheduler::checkpoint::Checkpoint` 保存到 Redis
- **AND** `scheduler::checkpoint::Memory` 可以从同一个 Redis checkpoint 持久化实现恢复任务状态

#### Scenario: Redis durable scheduler backend is available

- **WHEN** 调用方使用内置 `scheduler::Redis`
- **THEN** 它直接以 Redis 持久化 ready、delayed 与 inflight 任务状态
- **AND** 它继续遵守共享的 task identity、priority、depth 与 requeue 语义

#### Scenario: Custom scheduler and checkpoint backends remain extensible

- **WHEN** 调用方需要自定义 scheduler 或 checkpoint 持久化后端
- **THEN** 它可以分别实现 `scheduler::Scheduler` 或 `scheduler::checkpoint::Persist`
- **AND** 继续复用共享的 task state 与 checkpoint 边界

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

#### Scenario: Engine prefers batch store writes for kept items from one output

- **WHEN** 同一次 `parse()` / callback 输出里有多个通过 pipeline 的 items
- **THEN** 引擎优先调用一次 `store.batch_write(...)`
- **AND** 不再默认对这一批 item 分别重复调 `store.write(...)`

#### Scenario: Default batch store implementation falls back to single writes

- **WHEN** 某个 store 没有覆盖 `batch_write(...)`
- **THEN** 默认实现按顺序逐条调用 `write(...)`
- **AND** 简单 store 仍然可以只实现单条写入路径

#### Scenario: Store remains the unified final output path for databases files APIs and queues

- **WHEN** 调用方需要把 item 写入数据库、文件、HTTP API 或消息队列
- **THEN** 这些最终输出继续挂在同一个 `store` 边界上
- **AND** 框架不再为外部输出引入另一套独立 sink runtime

#### Scenario: Default engine store writes JSON Lines output

- **WHEN** 调用方未显式设置 `with_store(...)`
- **THEN** 引擎默认使用 `store::File::default()`
- **AND** 最终输出写入 `output/<spider_name>.jsonl`

#### Scenario: SQLite store creates tables and stores mapped item fields

- **WHEN** 调用方使用内置 `store::Sqlite`
- **THEN** store 自动创建目标 SQLite 表，并为每条 item 写入完整 `item_json`
- **AND** 显式声明的字段列按对应列类型写入数据库

#### Scenario: SQLite store rejects incompatible mapped field values explicitly

- **WHEN** `store::Sqlite` 的显式字段列与 item 值类型不兼容
- **THEN** store 返回显式错误，而不是静默转换或丢弃该列

#### Scenario: Webhook store pushes item JSON through the same store boundary

- **WHEN** 调用方使用内置 `store::Webhook`
- **THEN** store 把完整 item JSON 推送到配置的 HTTP endpoint
- **AND** 如果目标接口返回非 `2xx`，store 返回显式错误

#### Scenario: Redis store pushes item JSON through the same store boundary

- **WHEN** 调用方使用内置 `store::Redis`
- **THEN** store 把完整 item JSON 通过 `SADD` 写入目标 Redis set
- **AND** 如果 Redis 返回 error reply，store 返回显式错误

#### Scenario: Redis store batches multiple item JSON values through one SADD

- **WHEN** 引擎对内置 `store::Redis` 调用 `batch_write(...)`
- **THEN** store 把这批完整 item JSON 合并进同一个 `SADD key value...` 命令

#### Scenario: Kafka store pushes item JSON through the same store boundary

- **WHEN** 调用方使用内置 `store::Kafka`
- **THEN** store 把完整 item JSON 作为消息 value 写入目标 Kafka topic
- **AND** 如果 Kafka producer 返回投递错误，store 返回显式错误

#### Scenario: Kafka store batch write sends multiple item JSON messages

- **WHEN** 引擎对内置 `store::Kafka` 调用 `batch_write(...)`
- **THEN** store 会在同一次 store 调用里连续发送多条 item JSON 消息到同一个 topic

### Requirement: Engine Exposes Minimal Runtime Stats

系统 MUST 提供最小运行时计数快照，覆盖核心请求、响应、重试、错误与 item 统计。

#### Scenario: Runtime stats snapshot includes core counters

- **WHEN** 调用方读取 `engine.stats()`
- **THEN** 返回的快照包含 `request_count`、`response_count`、`error_count`、`retry_count`、`item_count` 与 `pipeline_drop_count`

#### Scenario: Dropped items do not inflate final item count

- **WHEN** 某个 item 被 pipeline 显式丢弃
- **THEN** `pipeline_drop_count` 增加
- **AND** `item_count` 不增加

#### Scenario: Stats remain cumulative for the engine instance

- **WHEN** 同一个 engine 实例连续执行多次任务
- **THEN** `engine.stats()` 返回累计计数，而不是只保留最近一次任务的局部结果

### Requirement: Engine Supports A Minimal robots.txt Policy

系统 MUST 提供最小 `robots.txt` 抓取策略，并明确默认关闭与当前受限边界。

#### Scenario: robots.txt policy stays disabled unless enabled explicitly

- **WHEN** 调用方未显式开启 robots 策略
- **THEN** 引擎不会额外因为 `robots.txt` 拦截请求

#### Scenario: Disallowed requests are skipped before download

- **WHEN** 调用方通过 `Settings::with_robots_obey(true)` 开启 robots 策略
- **AND** 当前 origin 的 `robots.txt` 不允许该请求路径
- **THEN** 引擎在真正下载前跳过该请求

#### Scenario: robots user-agent defaults to spider name

- **WHEN** 调用方开启 robots 策略但未显式设置 robots user-agent
- **THEN** 引擎使用 `spider.name()` 作为 robots 匹配 user-agent

#### Scenario: Minimal robots status handling remains explicit

- **WHEN** `robots.txt` 返回 `404`
- **THEN** 当前 origin 视为允许抓取
- **AND** 当 `robots.txt` 返回 `401` 或 `403` 时，当前 origin 视为拒绝抓取
- **AND** 其它抓取失败或非成功状态当前保持 fail-open

### Requirement: HTTP Downloader Wires Shared Transport Semantics

系统 MUST 在 HTTP downloader 中把 timeout、cookie jar、proxy 与 redirect 统一接到共享 request 语义上。

#### Scenario: Session requests share cookie jar state

- **WHEN** 多个请求复用同一个 session 标识
- **THEN** 前一跳响应写入的 cookie 会进入同一 cookie jar，并作用到后续请求

#### Scenario: Request timeout and proxy settings are executed by downloader

- **WHEN** HTTP request 显式声明 timeout、proxy 或 redirect 行为
- **THEN** downloader 依据这些 request 语义执行真实网络行为，而不是忽略或分散到不一致实现中
