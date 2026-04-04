# Runtime 与 Engine 规范

## 目标

定义 `Settings`、运行时编译和引擎主循环的行为，让 Spider 只关注抓取逻辑，而 Engine 负责执行、并发和生命周期。

### Requirement: Settings 拥有引擎级执行策略

库必须把 runtime 调优能力放在 `Settings` 上，而不是放在 `Spider` trait 上。

#### Scenario: Settings 推导 runtime 默认值

- Given 用户在 `Settings` 上配置了 download delay、retry code、retry count 与 dedup 行为
- When 引擎构建 runtime 配置
- Then 这些值会被转换成归一化的 runtime config

#### Scenario: 显式 runtime override 优先

- Given 调用了 `Settings::with_runtime()`
- When 请求 runtime 配置
- Then 显式传入的 runtime config 覆盖推导出的默认值

#### Scenario: Settings carries connection pool and OpenAI defaults

- Given 用户未显式修改连接池或 OpenAI 相关配置
- When 引擎与相关能力读取 Settings
- Then 系统提供稳定的默认 `connection_pool_size`、`openai_model` 与对应环境变量入口

### Requirement: Runtime config 可编译成 middleware 所需行为

库必须把 runtime 策略表示为 `schedule`、`retry` 与 `dedup` 三组 map，并且它们可以编译成 middleware 配置。

#### Scenario: retry 配置转换成 retry middleware 输入

- Given 存在 runtime retry 配置
- When 运行 runtime 编译过程
- Then 引擎能够为 retry 行为生成 middleware 配置

#### Scenario: 显式 middleware 可以覆盖 runtime 派生默认值

- Given 同时存在 runtime 派生的 middleware 与显式 middleware 配置
- When 引擎合并两者
- Then 相同 key 下由显式 middleware 配置优先

### Requirement: Engine 是持久运行的执行器

库必须让 `Engine::run()` 持续运行，直到收到显式 stop 信号。

#### Scenario: scheduler 为空不会终止引擎

- Given scheduler 暂时没有 ready task
- When 引擎进入空闲状态
- Then 引擎等待更多工作，而不是自动退出

#### Scenario: Shutdown handle 停止引擎

- Given 调用方持有 `shutdown_handle()`
- When 在该 handle 上调用 `stop()`
- Then 引擎完成进行中的工作并退出运行循环

### Requirement: Engine 应用并发与域名控制

库必须遵守 `Settings` 中的全局并发与按域名并发控制。

#### Scenario: 全局并发上限控制任务执行

- Given 引擎排队中的工作量超过 `concurrent_requests`
- When 开始执行任务
- Then 同时运行的任务数量不超过配置的全局上限

#### Scenario: 同域名请求受每域名上限约束

- Given 某个域名的并发请求数达到 `concurrent_requests_per_domain`
- When 引擎继续调度同一域名的新请求
- Then 这些请求等待该域名的并发槽位释放，而不是继续立即执行

#### Scenario: 全局并发与域名并发同时生效

- Given 同时配置了全局并发上限和每域名并发上限
- When 引擎调度任务
- Then 两个限制同时生效，并以更严格的限制为准

#### Scenario: allowed domains 过滤后续请求

- Given spider 返回了 `allowed_domains()`
- When 引擎准备把域名不在白名单中的请求入队
- Then 该请求在进入 scheduler 前被拒绝

### Requirement: Engine 通过 pipeline 和 store 处理 items

库必须在启动时打开配置好的 pipeline 和 store，并在运行循环中按
`parse -> item -> pipeline -> store` 这条主链处理 spider 的输出。

#### Scenario: Pipeline 以 spider 名称打开

- Given 引擎启动某个 spider
- When 运行循环开始
- Then pipeline 以该 spider 名称打开

#### Scenario: Store 以 spider 名称打开

- Given 引擎启动某个 spider
- When 运行循环开始
- Then store 以该 spider 名称打开

#### Scenario: 输出包含 items 与后续请求

- Given 某个回调或 DSL step 返回了输出
- When 引擎处理该输出
- Then items 先进入 pipeline
- And 被 pipeline 保留的 items 继续进入 store
- And requests 回到调度流程

#### Scenario: Pipeline 可以显式丢弃 item

- Given 某个 pipeline 对 item 返回 `Ok(false)`
- When 引擎处理该 item
- Then 该 item 不再进入 store
- And 该 item 不再进入最终输出集合

#### Scenario: Pipeline 错误会显式失败当前任务

- Given 某个 pipeline 在处理 item 时返回错误
- When 引擎处理该 item
- Then 当前任务显式失败，而不是依赖隐式 best effort

#### Scenario: Store 错误会显式失败当前任务

- Given 某个 store 在写入 item 时返回错误
- When 引擎处理该 item
- Then 当前任务显式失败，而不是静默忽略写入失败

#### Scenario: Engine prefers batch store writes for kept items from one output

- Given 某个回调或 DSL step 一次产出了多个 item
- And 这些 item 都通过了 pipeline
- When 引擎处理这批 item
- Then 引擎优先调用一次 `store.batch_write(...)`
- And 默认不会对这一批 item 分别重复调 `store.write(...)`

#### Scenario: Default batch store implementation falls back to single writes

- Given 某个 store 只实现了 `write(...)`
- When 引擎调用该 store 的 `batch_write(...)`
- Then 默认实现会按顺序逐条调用 `write(...)`
- And 简单 store 不需要为了接入引擎额外实现批量写入

#### Scenario: Store is the unified final output path for databases files APIs and queues

- Given 调用方需要把 item 写入数据库、文件、HTTP API 或消息队列
- When 它扩展或组合框架输出能力
- Then 这些最终输出都继续挂在同一个 `store` 边界上
- And 框架不再为这些外部输出额外引入另一套独立 sink runtime

#### Scenario: Default engine store writes JSON Lines to output directory

- Given 调用方没有显式设置 `with_store(...)`
- When 引擎运行某个 spider
- Then 引擎默认使用 `store::File::default()`
- And 最终输出写入 `output/<spider_name>.jsonl`

#### Scenario: SQLite store creates tables and stores mapped item fields

- Given 调用方使用内置 `store::Sqlite`
- When 引擎启动并处理 item
- Then store 会自动创建目标 SQLite 数据库表
- And 每条 item 至少写入 `spider_name` 与完整 `item_json`
- And 显式声明的字段列按对应列类型写入数据库

#### Scenario: SQLite store rejects incompatible mapped field values explicitly

- Given `store::Sqlite` 为某个字段声明了显式列类型
- When item 中该字段的值与列类型不兼容
- Then store 返回显式错误
- And 引擎不会静默把值写成另一种 SQLite 表示

#### Scenario: Webhook store pushes item JSON through the same store boundary

- Given 调用方使用内置 `store::Webhook`
- When store 写入某个 item
- Then 它把完整 item JSON 通过 HTTP 推送到配置的 endpoint
- And 如果目标接口返回非 `2xx`，store 返回显式错误

#### Scenario: Redis store pushes item JSON through the same store boundary

- Given 调用方使用内置 `store::Redis`
- When store 写入某个 item
- Then 它把完整 item JSON 通过 `SADD` 写入目标 Redis set
- And 如果 Redis 返回 error reply，store 返回显式错误

#### Scenario: Redis store can batch multiple item JSON values into one SADD

- Given 调用方使用内置 `store::Redis`
- And 某次输出里有多个通过 pipeline 的 items
- When 引擎调用 `store.batch_write(...)`
- Then store 把这批完整 item JSON 合并进同一个 `SADD key value...` 命令

#### Scenario: Kafka store pushes item JSON through the same store boundary

- Given 调用方使用内置 `store::Kafka`
- When store 写入某个 item
- Then 它把完整 item JSON 作为消息 value 写入目标 Kafka topic
- And 如果 Kafka producer 返回投递错误，store 返回显式错误

#### Scenario: Kafka store batch write sends multiple item JSON messages

- Given 调用方使用内置 `store::Kafka`
- And 某次输出里有多个通过 pipeline 的 items
- When 引擎调用 `store.batch_write(...)`
- Then store 会在同一次 store 调用里连续发送多条 item JSON 消息到同一个 topic

#### Scenario: Custom store implementations plug into the same final item chain

- Given 调用方自己实现了 `store::Store`
- When 它通过 `Engine::with_store(...)` 挂到引擎上
- Then 自定义 store 仍然走同一条 `parse -> item -> pipeline -> store` 主链
- And 如果它覆盖了 `batch_write(...)`，引擎也会优先使用该批量路径

### Requirement: Engine exposes minimal runtime stats

库必须提供最小运行时计数快照，方便调用方读取核心执行计数。

#### Scenario: Engine reports request response retry error and item counters

- Given 引擎已经执行过请求、重试、item 处理与错误路径
- When 调用方读取 `engine.stats()`
- Then 返回的快照包含 `request_count`、`response_count`、`error_count`、`retry_count`、`item_count` 与 `pipeline_drop_count`

#### Scenario: Stats count only items that were written after pipeline

- Given 某个 item 被 pipeline 显式丢弃
- When 调用方读取 `engine.stats()`
- Then `pipeline_drop_count` 增加
- And `item_count` 不增加

#### Scenario: Stats are cumulative for one engine instance

- Given 同一个 engine 实例连续执行了多次任务
- When 调用方读取 `engine.stats()`
- Then 快照中的计数是该 engine 实例生命周期内的累计值

### Requirement: Engine supports a minimal robots.txt crawl policy

库必须提供最小 `robots.txt` 抓取策略，并明确默认行为与当前受限边界。

#### Scenario: robots.txt policy is disabled by default

- Given 调用方没有显式开启 robots 策略
- When 引擎执行请求
- Then 引擎不会额外因为 `robots.txt` 阻止请求

#### Scenario: Engine skips a disallowed request before download

- Given 调用方通过 `Settings::with_robots_obey(true)` 开启 robots 策略
- And 当前 origin 的 `robots.txt` 禁止访问该请求路径
- When 引擎准备下载该请求
- Then 引擎在真正下载前跳过该请求

#### Scenario: robots user-agent falls back to spider name

- Given 调用方开启 robots 策略但没有显式设置 robots user-agent
- When 引擎检查 `robots.txt`
- Then 引擎使用当前 `spider.name()` 作为 robots 匹配 user-agent

#### Scenario: Minimal robots fetch failure remains fail-open except explicit deny statuses

- Given 调用方开启 robots 策略
- When `robots.txt` 返回 `404`
- Then 当前 origin 视为允许抓取
- And 当 `robots.txt` 返回 `401` 或 `403` 时，当前 origin 视为拒绝抓取
- And 其它抓取失败或非成功状态当前保持 fail-open

### Requirement: Scheduler 以 task identity 跟踪任务生命周期

库必须使用稳定的 task identity 跟踪 ready、delayed、inflight 与 retry 任务，而不是只依赖 URL。

#### Scenario: Same URL requests can be acked independently

- Given 两个请求 URL 相同，但 method、body 或 meta 不同
- When 它们先后进入 inflight 并被 ack 或 nack
- Then scheduler 能够独立处理这两个任务，而不会因为 URL 相同误删或误重排

#### Scenario: Retry preserves the original task identity

- Given 某个 inflight task 因错误被重试或延迟重排
- When scheduler 重新接收该任务
- Then 该任务沿用原始 task identity，而不是生成一个新的 URL 级占位标识

#### Scenario: Memory scheduler exposes its scheduler state as ready delayed inflight state

- Given 当前使用的是 `scheduler::Memory`
- When 调用方导出 scheduler checkpoint
- Then 快照显式包含 `ready`、`delayed` 与 `inflight` 三组任务状态
- And 这三个状态就是当前代码里的 scheduler state 对应物

#### Scenario: Durable scheduler checkpoint implementations restore from a shared checkpoint

- Given 调用方需要把 scheduler 状态持久化到磁盘、SQLite、Redis 或其它存储
- When 它实现 durable scheduler checkpoint 能力
- Then 它基于共享的 `scheduler::checkpoint::Checkpoint` 边界读写状态
- And 当前库不把 `scheduler::Memory` 误承诺为 crash-safe durable scheduler

#### Scenario: File scheduler checkpoint persistence stores and restores checkpoints

- Given 调用方使用内置 `scheduler::checkpoint::File`
- When 它保存或恢复 `scheduler::checkpoint::Checkpoint`
- Then 快照会持久化到文件
- And `scheduler::checkpoint::Memory` 可以基于同一个文件 checkpoint 持久化实现恢复之前的任务状态

#### Scenario: Redis scheduler checkpoint persistence stores and restores checkpoints

- Given 调用方使用内置 `scheduler::checkpoint::Redis`
- When 它保存或恢复 `scheduler::checkpoint::Checkpoint`
- Then 快照会持久化到 Redis key
- And `scheduler::checkpoint::Memory` 可以基于同一个 Redis checkpoint 持久化实现恢复之前的任务状态

#### Scenario: Redis scheduler directly implements durable task lifecycle semantics

- Given 调用方使用内置 `scheduler::Redis`
- When 它执行 enqueue、take_ready、complete 或 requeue
- Then 任务状态会直接持久化在 Redis 中
- And Redis scheduler 继续遵守 `ready / delayed / inflight`、`priority`、`depth` 与 stable task identity 的共享语义

#### Scenario: Custom scheduler and checkpoint backends reuse the same runtime boundary

- Given 调用方需要自定义 scheduler 或 checkpoint 持久化后端
- When 它分别实现 `scheduler::Scheduler` 或 `scheduler::checkpoint::Persist`
- Then 引擎仍然复用同一个 task state 与 checkpoint 边界

#### Scenario: Ready task order prefers priority then depth

- Given 多个 ready task 同时进入 `scheduler::Memory`
- When scheduler 选择下一个 ready task
- Then 更高 `priority` 的任务先被取出
- And 在 `priority` 相同的情况下，更低 `depth` 的任务先被取出
- And 如果 `priority` 与 `depth` 都相同，则保持 FIFO 顺序

#### Scenario: Persistent memory scheduler saves every state transition

- Given 调用方使用 `scheduler::checkpoint::Memory` 与共享 `scheduler::checkpoint::Persist`
- When enqueue、take_ready、complete 或 requeue 改变 scheduler state
- Then 当前 scheduler checkpoint 会被保存到对应的 checkpoint 持久化实现
- And 下次启动时可以从同一个 checkpoint 持久化实现恢复之前的 scheduler state

### Requirement: HTTP Downloader Applies Shared Transport Request Semantics

库必须在 HTTP downloader 中统一接线 timeout、cookie jar、proxy 与 redirect 能力，而不是分别散落为不一致的临时实现。

#### Scenario: Connection pool size comes from Settings

- Given 用户在 `Settings` 中显式配置了 `connection_pool_size`
- When 引擎创建 HTTP downloader 或其底层客户端
- Then 连接池大小使用该配置值

#### Scenario: Per-request timeout aborts download explicitly

- Given 某个 HTTP request 显式声明了 timeout
- When 下载时间超过该 timeout
- Then downloader 返回显式 download error，而不是无限等待或静默忽略 timeout

#### Scenario: Session requests reuse the same cookie jar

- Given 两个 HTTP request 共享同一个 session 标识
- When 第一个响应写入 cookie，第二个请求继续访问同一站点
- Then 第二个请求会复用同一 cookie jar，而不是丢失前一跳写入的 cookie

#### Scenario: Proxy and redirect policy come from request semantics

- Given 某个 HTTP request 显式声明了 proxy 或 redirect 行为
- When downloader 执行该请求
- Then 它使用同一套 request 语义决定代理路由与是否跟随重定向
