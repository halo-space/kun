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

#### Scenario: Redis durable scheduler reclaims stale inflight tasks by lease timeout

- **WHEN** 某个 task 已经进入 `inflight`
- **AND** 原 worker 在 lease timeout 内没有完成或重排这个任务
- **THEN** 后续 worker 访问同一个 Redis scheduler namespace 时，会把该 stale `inflight` task 回收到 `ready` 或 `delayed`
- **AND** 任务继续沿用原始 task identity

#### Scenario: Redis durable scheduler validates worker ownership before resolving a lease

- **WHEN** 某个 worker 已经 claim 一条 task 并拿到 lease
- **AND** 另一个 worker 试图用不同 worker identity 完成或重排这条 task
- **THEN** scheduler 会拒绝这次 complete 或 requeue
- **AND** 旧 lease 不会覆盖当前 inflight owner

#### Scenario: Engine renews Redis durable scheduler leases while long tasks are still running

- **WHEN** 调用方给 `scheduler::Redis` 配置 `lease_timeout` 与 `heartbeat_interval`
- **AND** 某个 task 的实际处理时间长于第一次 lease timeout 窗口
- **THEN** engine 在任务仍然运行时会继续 heartbeat 当前 lease
- **AND** 其它 worker 不会把这条 task 提前回收到 `ready` 或 `delayed`

#### Scenario: Durable scheduler snapshot remains an operational view

- **WHEN** 调用方通过 `scheduler::Sqlite::snapshot()` 或 `scheduler::Redis::snapshot()` 读取某个 scope 的即时状态
- **THEN** 它除了 ready / delayed / inflight 聚合计数外，还能看到当前 inflight ownership 明细
- **AND** 这份视图继续表示 durable scheduler 的当前运行态，而不是 checkpoint 快照

#### Scenario: Checkpoint restore does not pretend to be runtime reclaim

- **WHEN** 调用方通过 `scheduler::checkpoint::Memory` 从 checkpoint 恢复状态
- **AND** checkpoint 里本来就保存了 `inflight` task
- **THEN** 恢复结果仍然只是当时那份 `ready / delayed / inflight` 快照
- **AND** checkpoint 不会额外承担 durable scheduler 的 lease reclaim 语义

#### Scenario: Custom scheduler and checkpoint backends remain extensible

- **WHEN** 调用方需要自定义 scheduler 或 checkpoint 持久化后端
- **THEN** 它可以分别实现 `scheduler::Scheduler` 或 `scheduler::checkpoint::Persist`
- **AND** 当前内置 scheduler 后端至少包含 `Memory`、`Redis` 与 `Sqlite`
- **AND** 它们继续复用共享的 task state、worker、control 与 checkpoint 边界

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

### Requirement: Engine Defines Explicit Scheduler Transaction Boundaries

系统 MUST 明确任务执行完成后 `store`、follow/retry enqueue 与 scheduler resolve 的提交边界，而不是只依赖“单条 scheduler 状态迁移原子”来隐含整个引擎事务语义。

#### Scenario: Successful task completion has an explicit commit boundary

- **WHEN** 某条任务成功执行并产出 item 与 follow request
- **THEN** 系统对 `store`、follow enqueue 与 scheduler complete 的顺序和失败语义是明确、可测试的
- **AND** 调用方可以从文档和运行结果中理解这条路径的 at-least-once 边界

#### Scenario: Store failure does not silently commit scheduler completion

- **WHEN** 某条任务已经完成解析，但 `store` 写入失败
- **THEN** 系统不会把该任务静默当成已成功完成
- **AND** 调用方可以稳定区分这是 store failure，而不是 scheduler completion

#### Scenario: Scheduler resolve failure after store commit stays diagnosable

- **WHEN** 某条任务已经成功写入 `store`，但后续 `scheduler complete / complete-and-enqueue` 失败
- **THEN** 已写出的 item 不会被回滚
- **AND** 调用方可以通过测试、文档与日志明确理解这条 at-least-once 边界

### Requirement: Shared scheduler trait supports batch operations

系统 MUST 为共享 `scheduler::Scheduler` trait 提供统一 batch 调度接口，并统一使用 `batch` 命名，而不是只让高吞吐路径依赖单条接口并发堆叠。

#### Scenario: Backends can claim ready tasks in batch

- **WHEN** 调用方对某个 scheduler 请求一批 ready task
- **THEN** `Memory`、`Sqlite`、`Redis` 以及后续其它后端都可以通过统一 batch API 返回这批 claim 结果

#### Scenario: Backends can resolve multiple leases in one batch API

- **WHEN** 调用方需要批量 complete、requeue，或 complete-and-enqueue 多条任务
- **THEN** 共享 scheduler trait 提供统一的 batch 入口
- **AND** 默认实现允许旧后端先回退到单条接口循环执行

### Requirement: Scheduler operations plane is distinct from batch execution

系统 MUST 把跨 job 运维控制动作与 batch 执行接口区分开，不把 `pause / resume / release / purge` 这类运维动作混入 batch 调度 API。

#### Scenario: Control actions are not modeled as batch task resolution

- **WHEN** 调用方需要对 scope、worker 或 job 执行运维动作
- **THEN** 这些动作通过独立的运维控制入口暴露
- **AND** 它们不与 `take_batch_ready / complete_batch / requeue_batch` 混成同一组接口

### Requirement: Cross-job scheduler control is explicit

系统 MUST 为多 job / 多 scope 的 scheduler 运行提供统一、明确的运维控制入口，而不是只提供快照读取。

#### Scenario: Multiple scopes can be inspected through one control interface

- **WHEN** 调用方在同一个 backend 上运行多个 job / scope
- **THEN** 它可以通过统一入口读取 scopes、overview、workers 等跨 job 视图

#### Scenario: Cross-job control actions have concrete examples

- **WHEN** 调用方需要暂停、恢复、释放 worker 持有任务，或清理某个 scope
- **THEN** 文档和示例提供具体操作方式
- **AND** 这些动作的边界与影响范围是明确、可测试的

### Requirement: Durable scheduler snapshot exposes inflight ownership details

系统 MUST 让内置 durable scheduler 的运行时 snapshot 不仅返回 scope 级聚合计数，也能返回当前 inflight task 的 ownership / lease / deadline 明细。

#### Scenario: Durable scheduler snapshot shows inflight lease ownership

- **WHEN** 某个 `scheduler::Sqlite` 或 `scheduler::Redis` scope 当前存在 inflight task
- **THEN** `snapshot()` 返回的结构里包含这条 inflight task 的 task identity、worker identity、lease identity 与当前 deadline
- **AND** 调用方不需要手工回读底层后端状态才能定位 ownership 状态

#### Scenario: Cross-scope snapshots keep per-task ownership visibility

- **WHEN** 调用方读取 `snapshots_with_prefix(...)`
- **THEN** 每个 scope snapshot 都继续包含对应 inflight task 的 ownership / lease 明细
- **AND** 这些明细与 scope 级 ready / delayed / inflight 聚合计数保持一致

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

#### Scenario: File store can rotate output into numbered files

- **WHEN** 调用方对内置 `store::File` 使用 `with_rotate_items(...)` 或 `with_rotate_bytes(...)`
- **THEN** store 按阈值把输出切分到编号文件
- **AND** 默认命名保持在同一个基础路径上追加序号，例如 `items-0001.jsonl`

#### Scenario: File store can switch to a readable pretty block format

- **WHEN** 调用方对内置 `store::File` 使用 `with_format(store::FileFormat::PrettyJsonBlocks)`
- **THEN** store 继续写同一条最终 item 链路
- **AND** 每条 item 以可读的 pretty JSON block 形式落盘

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

#### Scenario: Webhook store retries retryable failures with explicit backoff

- **WHEN** 调用方对内置 `store::Webhook` 设置 `with_retry_limit(...)` 与 `with_retry_backoff(...)`
- **AND** 请求错误或目标接口返回 `429 / 5xx`
- **THEN** store 按配置的 backoff 重试
- **AND** 其它非 `2xx` 继续直接返回显式错误

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

#### Scenario: Kafka store can attach message key and headers

- **WHEN** 调用方对内置 `store::Kafka` 使用 `with_key(...)`、`with_key_field(...)`、`with_header(...)` 或 `with_header_field(...)`
- **THEN** store 在继续写完整 item JSON value 的同时，附带对应的 message key 与 headers
- **AND** 从 item 字段取值失败时返回显式错误

#### Scenario: Kafka store batch write sends multiple item JSON messages

- **WHEN** 引擎对内置 `store::Kafka` 调用 `batch_write(...)`
- **THEN** store 会在同一次 store 调用里连续发送多条 item JSON 消息到同一个 topic

#### Scenario: Built-in store maintenance scope remains explicit

- **WHEN** 调用方需要 PostgreSQL、对象存储、复杂第三方 API 或更高阶 MQ 语义
- **THEN** 框架继续建议通过自定义 `store::Store` 扩展
- **AND** 当前内置维护范围明确保持在 `Memory / File / Sqlite / Webhook / Redis / Kafka`

### Requirement: Request Execution Policies Run On Explicit Lifecycle Boundaries

系统 MUST 把 request-scoped execution policy 放在明确的生命周期边界执行，而不是继续混成 engine 全局组件或模糊的 `schedule` 语义。

#### Scenario: Request execution policy order is deterministic

- **WHEN** 一条新 request 被发现，并最终完成一次 download attempt
- **THEN** 系统按固定顺序执行这些边界：
- **AND** 先在 admission 阶段执行 allowed-domain 检查与 request dedup
- **AND** request 进入 scheduler 并被 claim 后，再在 download attempt 前执行 download-before middleware
- **AND** download 返回错误或 retryable response 后，才执行 retry middleware

#### Scenario: Admission policies run before a request enters the scheduler

- **WHEN** start request、follow request、manual enqueue request 或其它新发现 request 准备进入 scheduler
- **THEN** 系统先执行 request admission 边界上的策略
- **AND** 这条边界至少明确包含 allowed-domain 检查与 request dedup

#### Scenario: Download-before middleware runs before each download attempt

- **WHEN** 某条 task 已被 claim，并即将发起一次 download attempt
- **THEN** 系统先执行当前 request 的 effective download-before middleware
- **AND** 如果该 attempt 需要退避，任务会带 delay 回到 scheduler，而不是阻塞 worker

#### Scenario: Delay is distinct from retry

- **WHEN** 某条 request 在 download 前因为 `concurrency`、`interval`、`rate_limit`、`auto_throttle`、crawl-delay 或其它同类原因被延迟
- **THEN** 系统把它视为一次 `Delay`
- **AND** 这次事件不会增加 retry 次数
- **AND** 它会回到 scheduler delayed bucket，而不是走 retry 计数路径

#### Scenario: Retry runs after a failed download attempt or retryable response

- **WHEN** download 返回错误，或 response 命中 retry policy
- **THEN** 系统在这次 attempt 之后执行 request 的 effective retry policy
- **AND** 如果命中 retry，下一次 attempt 继续沿用同一条 request 的有效运行时上下文
- **AND** retry 不会倒置到 download-before middleware 或 dedup 之前执行

### Requirement: Middleware Lifecycle Uses Object-Scoped Flow And Context Types

系统 MUST 按对象生命周期组织 middleware 的 flow 与 context，而不是继续让所有 hook 共享一个总 `Flow` 和一个大一统上下文。

#### Scenario: Middleware flow families are scoped by lifecycle object

- **WHEN** 框架为 middleware 暴露控制流类型
- **THEN** 它至少区分 `enqueue`、`download`、`parse` 与 `item` 四类生命周期对象
- **AND** admission hook 只使用 enqueue flow
- **AND** download 相关 hook 共享 download flow
- **AND** parse 相关 hook 共享 parse flow
- **AND** item 相关 hook 共享 item flow

#### Scenario: Observational hooks do not pretend to be control-flow hooks

- **WHEN** 某个 hook 只是做收尾、副作用、日志、状态释放或埋点
- **THEN** 它返回普通结果而不是 flow
- **AND** 系统不会要求调用方为这类 hook 返回无意义的 `Continue`

#### Scenario: Contexts are object-scoped and event payload is passed separately

- **WHEN** middleware 在 download、parse 或 item 生命周期中执行
- **THEN** 框架按对象生命周期提供对应 context
- **AND** `response`、`error` 这类事件数据按需作为 hook 参数传入
- **AND** 框架不会继续把 request、response、error、item 强行混进一个充满可选字段的统一 context

### Requirement: Request Dedup Is A Request-Scoped Admission Policy

系统 MUST 把 dedup 建模成 request-scoped admission policy，而不是继续默认把所有 request 都挂在 engine 全局 dedup 组件上。

#### Scenario: Requests without dedup policy skip request dedup

- **WHEN** 某条 request 没有声明 dedup policy，且没有命中默认 request runtime dedup
- **THEN** 系统不会对它执行 request dedup
- **AND** 该 request 仍然可以继续走 allowed-domain 检查和后续调度

#### Scenario: Different requests can use different dedup policies in one spider run

- **WHEN** 同一轮 spider 运行里，列表页 request、详情页 request 或其它请求声明了不同 dedup policy
- **THEN** 系统按各自 request 的 effective dedup policy 做 admission 决策
- **AND** 不要求它们共享同一个 engine 全局 dedup 规则

#### Scenario: Internal retries are not rejected as fresh duplicates

- **WHEN** 某条 request 已经进入 retry 路径
- **THEN** 系统不会把这次内部 retry 当成一条新的外部发现 request 再次按原始 dedup policy 拒绝
- **AND** retry 路径的 admission 语义保持显式、可测试

### Requirement: Download-Before Middleware Uses Explicit Shared Buckets

系统 MUST 让 download-before middleware 基于显式 bucket 生效，而不是继续依赖 middleware instance 的匿名本地状态。

#### Scenario: Requests in the same bucket share limit state

- **WHEN** 两条 request 解析到同一个 limit bucket
- **THEN** 它们共享该 bucket 的 `concurrency`、`interval`、`rate_limit` 或 `auto_throttle` 状态

#### Scenario: Requests in different buckets do not accidentally share state

- **WHEN** 两条 request 解析到不同的 limit bucket
- **THEN** 它们的 limit state 相互隔离
- **AND** 系统不会因为复用同一个 step chain 或 middleware instance 就错误串桶

### Requirement: Engine Process Controls Stay Distinct From Request Policies

系统 MUST 把 engine worker/process 级控制，与 request-scoped execution policy 区分开。

#### Scenario: Global worker concurrency remains an engine throughput control

- **WHEN** 调用方配置 engine 的全局并发或 per-domain 并发
- **THEN** 这些值继续控制 worker 能同时 claim / 执行多少任务
- **AND** 它们不等价于某条 request 自己的 download-before middleware policy

#### Scenario: Download-before middleware and engine throughput controls can coexist

- **WHEN** engine 配了全局 worker 并发，同时某些 request 额外声明了更严格的 download-before middleware
- **THEN** 系统同时尊重这两层边界
- **AND** 不会把 request 级 download-before middleware 退化成 engine 全局吞吐开关

### Requirement: Engine Processes Spider Callback Outputs As Request-Scoped Work

系统 MUST 把 spider callback 返回的 request / item 收口回 engine 的固定执行边界，而不是让调用方自己推断后续执行顺序。

#### Scenario: Callback output requests re-enter admission after callback returns

- **WHEN** spider callback 通过 `Output { items, requests }` 返回了一条新的 request
- **THEN** engine 在 callback 返回后统一接管这条 request
- **AND** 它会重新进入 admission 边界，再按自己的 effective request runtime 执行 dedup / download-before middleware / retry middleware

#### Scenario: Callback output handling does not bypass runtime boundaries

- **WHEN** spider callback 返回 `Output { items, requests }`
- **THEN** 这些输出只表达“产出下一批工作”
- **AND** 它们不会绕过 scheduler、admission、download attempt 或 store/pipeline 这些既定 engine 边界

### Requirement: Request Middleware Resolution Uses Global, Step, And Request Layers

系统 MUST 以 engine global、current step default、current request override 三层来解析 request middleware，并且不允许 step 间或父子 request 间发生隐式覆盖继承。

#### Scenario: Request override wins over step and engine defaults

- **WHEN** 某条 request 显式给某个 middleware 写入 `Use(config)` 或 `Skip`
- **THEN** 该 request 的显式覆盖优先于当前 step 默认值与 engine 全局默认值

#### Scenario: Step default wins over engine global default

- **WHEN** 当前 step 给某个 middleware 配置了默认值，而 request 本身没有显式覆盖
- **THEN** 系统使用该 step 默认值
- **AND** 不再回退到 engine 全局默认值

#### Scenario: Middleware overrides do not inherit from parent request

- **WHEN** 某条 request 派生出 follow request 或 callback 中又构造出新的 request
- **THEN** 新 request 默认不继承父 request 的 middleware override
- **AND** 它只解析自己的 override、目标 step 默认值与 engine 全局默认值

### Requirement: Middleware Trait Uses Native Async Functions

系统 MUST 在 `Spider`、`Middleware` 与相关回调 trait 上使用 Rust 原生 `async fn in trait`，而不是依赖 `#[async_trait]` 宏。

#### Scenario: Middleware hooks use native async fn in trait

- **WHEN** 框架定义 middleware hook 或 spider callback trait
- **THEN** 这些 trait 使用 Rust 原生 `async fn in trait`
- **AND** 本次变更不引入 `#[async_trait]`

### Requirement: Downloaders Are Explicit Engine Components

系统 MUST 允许调用方显式替换 HTTP downloader 与 browser downloader，而不需要每次都重建全部 engine parts。

#### Scenario: Default engine uses built-in downloaders

- **WHEN** 调用方直接使用 `Engine::new()`
- **THEN** 引擎默认使用 `download::Http` 与 `download::Browser`

#### Scenario: Callers can replace only the HTTP downloader

- **WHEN** 调用方调用 `Engine::with_http(...)`
- **THEN** 引擎只替换 HTTP downloader
- **AND** 当前 browser downloader 保持不变

#### Scenario: Callers can replace only the browser downloader

- **WHEN** 调用方调用 `Engine::with_browser(...)`
- **THEN** 引擎只替换 browser downloader
- **AND** 当前 HTTP downloader 保持不变

#### Scenario: with_downloaders stays as a convenience shortcut

- **WHEN** 调用方调用 `Engine::with_downloaders(http, browser)`
- **THEN** 它继续表示“默认 memory scheduler + 一次替换两个 downloader”的快捷写法
- **AND** 其它默认 engine 组件保持不变

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

### Requirement: Engine Supports Minimal AutoThrottle

系统 MUST 继续提供最小 `AutoThrottle` 能力，但它应当属于 download-before middleware 语义，而不是继续伪装成 `runtime.schedule`。

#### Scenario: AutoThrottle is derived as a default download-before middleware policy

- **WHEN** 调用方在 `Config` 上开启 `with_auto_throttle(true)`
- **AND** 同时设置 `download_delay`、`with_auto_throttle_target_concurrency(...)` 与 `with_auto_throttle_max_delay(...)`
- **THEN** 系统把它归一化为默认 download-before middleware
- **AND** `download_delay` 继续表示起始/最小 delay，而不是另一条独立执行阶段

#### Scenario: AutoThrottle feedback stays inside the resolved limit bucket

- **WHEN** 同一个 bucket 最近请求变慢、返回 `429 / 5xx`，或下载直接失败
- **THEN** `auto_throttle` 只调整该 bucket 的后续 delay
- **AND** 不会错误影响其它 limit bucket

### Requirement: Engine Exposes Minimal Runtime Stats

系统 MUST 提供最小运行时计数快照，覆盖核心请求、响应、重试、错误与 item 统计。

#### Scenario: Runtime stats snapshot includes core counters

- **WHEN** 调用方读取 `engine.stats()`
- **THEN** 返回的快照包含 `request_count`、`response_count`、`error_count`、`retry_count`、`item_count` 与 `pipeline_drop_count`
- **AND** 也包含 `dedup_reject_count`、`robots_disallow_count`、`robots_delay_count`、`http_cache_hit_count`、`http_cache_revalidate_count`、`http_cache_store_count`、`http_cache_miss_count` 与 `store_error_count`

#### Scenario: Dropped items do not inflate final item count

- **WHEN** 某个 item 被 pipeline 显式丢弃
- **THEN** `pipeline_drop_count` 增加
- **AND** `item_count` 不增加

#### Scenario: Stats remain cumulative for the engine instance

- **WHEN** 同一个 engine 实例连续执行多次任务
- **THEN** `engine.stats()` 返回累计计数，而不是只保留最近一次任务的局部结果

#### Scenario: Minimal reporter hook extends stats without replacing snapshot API

- **WHEN** 调用方通过 `Engine::with_stats_reporter(...)` 注册自定义 reporter
- **THEN** `engine.stats()` 继续保持为主读取 API
- **AND** 每次累计计数更新时，引擎都会把对应 event 与最新 snapshot 推给 reporter
- **AND** 当前轮次不要求直接内置完整 Prometheus / OpenTelemetry exporter

### Requirement: Engine Exposes Minimal Signals And Extensions

系统 MUST 提供最小 runtime signal bus，让调用方可以监听生命周期与执行事件，并在同一条边界上挂扩展。

#### Scenario: Signal listeners can be registered explicitly

- **WHEN** 调用方通过 `Engine::with_signal_listener(...)` 注册自定义 listener
- **THEN** 引擎在 spider 生命周期与任务执行链路里发出的 runtime signals 会继续投递给这个 listener

#### Scenario: Extensions reuse the same signal bus

- **WHEN** 调用方通过 `Engine::with_extension(...)` 注册扩展
- **THEN** 这个 extension 会收到和 signal listener 相同的 runtime signals
- **AND** `with_extension(...)` 不会额外引入另一套独立 runtime

#### Scenario: Engine emits the minimal built-in signal set

- **WHEN** 引擎执行 spider 生命周期、request 调度、response 处理、item 写入或错误路径
- **THEN** 当前最小信号集合至少包含 `spider_opened`、`spider_closed`、`request_scheduled`、`response_received`、`item_scraped` 与 `spider_error`
- **AND** `spider_closed` 会携带最终 `stats::Snapshot`

### Requirement: Engine Supports A Minimal robots.txt Policy

系统 MUST 提供最小 `robots.txt` 抓取策略，并明确默认关闭与当前受限边界。

#### Scenario: robots policy is an explicit engine component

- **WHEN** 调用方调用 `Engine::with_robots(...)`
- **THEN** 引擎改用这个显式 robots 组件判断请求是否允许继续
- **AND** `Config::with_robots_obey(...)` 与 `Config::with_robots_user_agent(...)` 继续只负责启用开关与 user-agent 选择

#### Scenario: robots.txt policy stays disabled unless enabled explicitly

- **WHEN** 调用方未显式开启 robots 策略
- **THEN** 引擎不会额外因为 `robots.txt` 拦截请求

#### Scenario: Disallowed requests are skipped before download

- **WHEN** 调用方通过 `Config::with_robots_obey(true)` 开启 robots 策略
- **AND** 当前 origin 的 `robots.txt` 不允许该请求路径
- **THEN** 引擎在真正下载前跳过该请求

#### Scenario: Crawl-delay is enforced as a real runtime delay

- **WHEN** 调用方通过 `Config::with_robots_obey(true)` 开启 robots 策略
- **AND** 当前 origin 的 `robots.txt` 为匹配到的 user-agent group 声明了 `Crawl-delay`
- **THEN** 引擎会按该 delay 退避并重试同 origin 的后续请求
- **AND** 不会把这类请求误当成永久 `Disallow`

#### Scenario: Request-rate is enforced as a real runtime delay

- **WHEN** 调用方通过 `Config::with_robots_obey(true)` 开启 robots 策略
- **AND** 当前 origin 的 `robots.txt` 为匹配到的 user-agent group 声明了 `Request-rate`
- **THEN** 引擎会按 `window / requests` 计算出的均匀间隔最小 delay 退避并重试同 origin 的后续请求
- **AND** 如果同一个 group 同时声明了 `Crawl-delay` 与 `Request-rate`，当前取更严格的 delay

#### Scenario: robots user-agent defaults to spider name

- **WHEN** 调用方开启 robots 策略但未显式设置 robots user-agent
- **THEN** 引擎使用 `spider.name()` 作为 robots 匹配 user-agent

#### Scenario: Minimal robots status handling remains explicit

- **WHEN** `robots.txt` 返回 `404`
- **THEN** 当前 origin 视为允许抓取
- **AND** 当 `robots.txt` 返回 `401` 或 `403` 时，当前 origin 视为拒绝抓取
- **AND** 其它抓取失败或非成功状态当前保持 fail-open

#### Scenario: Temporarily unavailable robots fetches use a retry delay window

- **WHEN** 当前 origin 没有可用 robots cache
- **AND** 某次 `robots.txt` 抓取失败或返回临时非成功状态
- **THEN** 引擎先按当前 unavailable policy 处理这次请求
- **AND** 在 `unavailable_retry_delay` 窗口内，不会对同一个 origin 的每个请求都重复抓取 `robots.txt`

#### Scenario: Robots matching supports wildcard and group specificity

- **WHEN** 某个 robots policy 同时声明了多个 `User-agent` group 与带 `*` / `$` 的路径规则
- **THEN** 更具体的 group 优先于 wildcard group
- **AND** 路径匹配支持 `*` wildcard 与末尾 `$` end anchor

#### Scenario: Robots matching normalizes rule targets before path matching

- **WHEN** 某个 robots policy 的第一行带 UTF-8 BOM，或 `Allow` / `Disallow` 使用了 absolute URL / protocol-relative 规则值
- **THEN** 系统会先把这些规则值归一化到统一 URL 目标语义
- **AND** 只有 host 命中的 absolute 规则才会继续参与路径匹配

#### Scenario: Robots component can expose sitemap URLs

- **WHEN** 当前 origin 的 `robots.txt` 声明了一个或多个 `Sitemap`
- **THEN** robots 组件可以返回这些 sitemap URL

#### Scenario: Engine can turn robots sitemaps into seed requests

- **WHEN** 调用方开启 `Config::with_robots_sitemap_seeds(true)`
- **AND** 当前 origin 的 `robots.txt` 声明了一个或多个 `Sitemap`
- **THEN** 引擎会抓取这些 sitemap 文档，并把其中声明的页面 URL 自动加入种子请求集合
- **AND** 这些自动发现的种子请求仍然走引擎现有的 dedup 路径
- **AND** 当前实现保持默认 `priority = 0` 与 `depth = 0`

#### Scenario: Engine can turn gzipped robots sitemaps into seed requests

- **WHEN** 当前 origin 的 `robots.txt` 声明的 sitemap 是常见的 `.xml.gz` 压缩文档
- **THEN** 引擎仍然可以解析它并把里面的页面 URL 自动加入种子请求集合

#### Scenario: Engine can override robots sitemap seed priority and depth

- **WHEN** 调用方开启 `Config::with_robots_sitemap_seeds(true)`
- **AND** 它额外配置了 `with_robots_sitemap_seed_priority(...)` 或 `with_robots_sitemap_seed_depth(...)`
- **THEN** 引擎生成的 robots sitemap 种子请求会带上这些显式 `priority` / `depth`

#### Scenario: Robots sitemap requests inherit shared request semantics from start requests

- **WHEN** spider 通过 `build_start_requests()` 提供了带 cookies、proxy、session 或 browser mode 的起始请求
- **AND** 调用方开启 `Config::with_robots_sitemap_seeds(true)`
- **THEN** 引擎抓 sitemap 时继续继承这些共享请求语义，但强制走 HTTP 下载
- **AND** 由 sitemap 生成的页面种子请求继续继承对应 start request 的共享请求语义

#### Scenario: Default robots memory policy uses an in-memory cache backend

- **WHEN** 调用方使用默认 `robots::Memory` 并重复读取同一个 origin 的 policy
- **THEN** 默认 cache backend 是进程内的 `robots::cache::Memory`

#### Scenario: Callers can replace the robots cache backend

- **WHEN** 调用方保留 `robots::Memory` 这套 robots 语义
- **AND** 它通过 `robots::Memory::with_cache(...)` 提供自定义 cache backend
- **THEN** robots policy 的缓存读写会走这个显式 cache backend

#### Scenario: Built-in file robots cache persists entries across engine restarts

- **WHEN** 调用方使用 `robots::cache::File`
- **AND** 某个 origin 的 robots policy 被保存到这个 backend
- **THEN** 后续新的 engine 实例仍然可以从同一个 cache 文件恢复该 origin 的 robots policy

#### Scenario: Default robots cache uses a TTL-based refresh window

- **WHEN** 调用方使用默认 `robots::Memory`
- **AND** 某个 origin 的 robots policy 还在默认 `24h` 的 `cache_ttl` 内
- **THEN** 引擎会继续复用现有缓存，而不会每次都重新抓取 `robots.txt`

#### Scenario: Stale robots cache falls back to the previous policy on refresh failure

- **WHEN** 某个 origin 已经有过期的 robots cache 条目
- **AND** 引擎尝试刷新它，但这次抓取 `robots.txt` 失败或返回临时非成功状态
- **THEN** 引擎优先继续复用这条旧 cache policy
- **AND** 不会因为这次刷新失败直接把旧 policy 替换成新的 fail-open 缓存条目

#### Scenario: Robots memory policy can become strict when robots is unavailable

- **WHEN** 调用方对 `robots::Memory` 配置 `with_unavailable_policy(robots::UnavailablePolicy::DisallowAll)`
- **AND** 当前 origin 没有可用 robots cache
- **AND** 这次 `robots.txt` 抓取失败或返回临时非成功状态
- **THEN** 当前 origin 按拒绝抓取处理，而不是继续 fail-open

#### Scenario: Stale robots cache still has priority over strict unavailable policy

- **WHEN** 调用方对 `robots::Memory` 配置 `with_unavailable_policy(robots::UnavailablePolicy::DisallowAll)`
- **AND** 当前 origin 已有过期的 robots cache
- **AND** 这次刷新抓取失败或返回临时非成功状态
- **THEN** 引擎继续复用旧 cache policy
- **AND** 不会直接放弃旧缓存并改用新的 unavailable policy

#### Scenario: Callers can overlay explicit site matcher policy on robots memory

- **WHEN** 调用方保留内置 `robots::Memory`
- **AND** 它通过 `robots::Memory::with_site_policy(...)` 为某个 site matcher 配置 `robots::SitePolicy`
- **THEN** 这条显式站点策略会叠加在原始 `robots.txt` 语义之上

#### Scenario: More specific site matcher wins for access and unavailable handling

- **WHEN** 同一个请求同时命中多条显式 `robots::SitePolicy`
- **THEN** `SiteAccess` 与 `unavailable_policy` 采用更具体 matcher 的配置
- **AND** 如果 matcher specificity 相同，则后注册的规则优先

#### Scenario: Matched site policies merge delay and sitemap data

- **WHEN** 某个请求同时命中 `robots.txt` 规则和一条或多条显式 `robots::SitePolicy`
- **THEN** 最终 delay 取 robots delay 与所有命中 site policy delay 里更严格的那个
- **AND** 额外 sitemap 会与 robots sitemap 做去重合并

### Requirement: HTTP Downloader Wires Shared Transport Semantics

系统 MUST 在 HTTP downloader 中把 timeout、cookie jar、proxy 与 redirect 统一接到共享 request 语义上。

#### Scenario: Session requests share cookie jar state

- **WHEN** 多个请求复用同一个 session 标识
- **THEN** 前一跳响应写入的 cookie 会进入同一 cookie jar，并作用到后续请求

#### Scenario: Request timeout and proxy settings are executed by downloader

- **WHEN** HTTP request 显式声明 timeout、proxy 或 redirect 行为
- **THEN** downloader 依据这些 request 语义执行真实网络行为，而不是忽略或分散到不一致实现中

#### Scenario: Minimal HTTP cache adds conditional request headers from cached validators

- **WHEN** 调用方通过 `Config::with_http_cache(true)` 开启最小 HTTP cache
- **AND** 某个 HTTP `GET` 请求之前已经缓存了 `ETag` 或 `Last-Modified`
- **THEN** 后续同请求会自动补 `If-None-Match` 或 `If-Modified-Since`

#### Scenario: Minimal HTTP cache restores cached response on 304

- **WHEN** 调用方通过 `Config::with_http_cache(true)` 开启最小 HTTP cache
- **AND** 某个 HTTP `GET` 请求之前已经缓存了响应 body 与对应 validator
- **AND** 服务端对后续同请求返回 `304 Not Modified`
- **THEN** 引擎会回填缓存响应 body
- **AND** `Response.flags` 会包含 `http_cache`

#### Scenario: Default HTTP cache backend stays in memory unless replaced

- **WHEN** 调用方开启 HTTP cache 但没有显式替换 backend
- **THEN** 默认使用进程内 `middleware::http_cache::Memory`

#### Scenario: HTTP cache can persist entries through a file backend

- **WHEN** 调用方通过 `Config::with_http_cache_file(...)` 或 `HttpCache::with_cache(...)` 选择 `middleware::http_cache::File`
- **THEN** HTTP cache 条目会持久化到磁盘 JSON 文件

#### Scenario: ttl expiration turns stale entries into misses

- **WHEN** 某个 HTTP cache 条目超过配置的 `ttl`
- **THEN** 这条条目不会继续参与条件请求回源
- **AND** 引擎会把它统计为 cache miss

#### Scenario: HTTP cache exposes validator-only and response strategies

- **WHEN** 调用方把 `strategy` 设为 `validators`
- **THEN** 引擎只缓存 `ETag / Last-Modified`
- **AND** 服务端返回 `304 Not Modified` 时不会回填旧 body
- **WHEN** 调用方把 `strategy` 设为 `response`
- **THEN** 引擎还会缓存响应 body
- **AND** 服务端返回 `304 Not Modified` 时会回填旧 body

#### Scenario: HTTP cache stats include miss and store counters

- **WHEN** HTTP cache 发生 miss、revalidate、store 或 hit
- **THEN** `engine.stats()` 会累计 `http_cache_miss_count`、`http_cache_revalidate_count`、`http_cache_store_count` 与 `http_cache_hit_count`
