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

### Requirement: Engine 通过单一 pipeline 处理 items

库必须在启动时打开配置好的 pipeline，并在运行循环中用这条唯一的 item 管线处理 spider 的输出。

#### Scenario: Pipeline 以 spider 名称打开

- Given 引擎启动某个 spider
- When 运行循环开始
- Then pipeline 以该 spider 名称打开

#### Scenario: 输出包含 items 与后续请求

- Given 某个回调或 DSL step 返回了输出
- When 引擎处理该输出
- Then items 继续进入 pipeline，requests 回到调度流程

#### Scenario: Pipeline 可以显式丢弃 item

- Given 某个 pipeline 对 item 返回 `Ok(false)`
- When 引擎处理该 item
- Then 该 item 不再进入最终输出集合

#### Scenario: Pipeline 错误会显式失败当前任务

- Given 某个 pipeline 在处理 item 时返回错误
- When 引擎处理该 item
- Then 当前任务显式失败，而不是依赖隐式 best effort

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
- When 调用方导出 scheduler state 快照
- Then 快照显式包含 `ready`、`delayed` 与 `inflight` 三组任务状态
- And 这三个状态就是当前代码里的 scheduler state 对应物

#### Scenario: Durable scheduler state implementations restore from a shared state snapshot

- Given 调用方需要把 scheduler 状态持久化到磁盘、SQLite、Redis 或其它存储
- When 它实现 durable scheduler state/store 能力
- Then 它基于共享的 `scheduler::state::Snapshot` 边界读写状态
- And 当前库不把 `scheduler::Memory` 误承诺为 crash-safe durable scheduler

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
