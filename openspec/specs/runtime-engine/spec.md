# Runtime 与 Engine 规范

## 目标

定义 `Settings`、运行时编译和引擎主循环的行为，让 Spider 只关注抓取逻辑，而 Engine 负责执行、并发和生命周期。

### Requirement: Settings 拥有引擎级执行策略

库必须把 runtime 调优能力放在 `Settings` 上，而不是放在 `Spider` trait 上。

#### Scenario: Settings 推导 runtime 默认值

- Given 用户在 `Settings` 上配置了 download delay、retry code 与 retry count
- When 引擎构建 runtime 配置
- Then 这些值会被转换成归一化的 runtime config

#### Scenario: Settings can derive auto throttle defaults

- Given 用户在 `Settings` 上开启了 `auto_throttle`
- And 配置了 `download_delay`、`auto_throttle_target_concurrency` 与 `auto_throttle_max_delay`
- When 引擎构建 runtime 配置
- Then 这些值会被转换成 `auto_throttle` 所需的归一化 runtime schedule
- And `download_delay` 被用作 auto throttle 的起始/最小 delay，而不是继续单独派生成固定 interval gate

#### Scenario: 显式 runtime override 优先

- Given 调用了 `Settings::with_runtime()`
- When 请求 runtime 配置
- Then 显式传入的 runtime config 覆盖推导出的默认值

#### Scenario: Settings carries connection pool and OpenAI defaults

- Given 用户未显式修改连接池或 OpenAI 相关配置
- When 引擎与相关能力读取 Settings
- Then 系统提供稳定的默认 `connection_pool_size`、`openai_model` 与对应环境变量入口

### Requirement: Runtime config 可编译成 middleware 所需行为

库必须把 runtime 中与下载链路相关的 `schedule`、`retry` 策略编译成 middleware 配置。

#### Scenario: retry 配置转换成 retry middleware 输入

- Given 存在 runtime retry 配置
- When 运行 runtime 编译过程
- Then 引擎能够为 retry 行为生成 middleware 配置

#### Scenario: auto throttle 配置转换成 adaptive delay middleware 输入

- Given runtime schedule 里开启了 `auto_throttle`
- When 运行 runtime 编译过程
- Then 引擎能够为自适应限速生成 `auto_throttle` middleware 配置
- And 不再同时为同一份 schedule 额外生成固定 `interval_gate`

#### Scenario: 显式 middleware 可以覆盖 runtime 派生默认值

- Given 同时存在 runtime 派生的 middleware 与显式 middleware 配置
- When 引擎合并两者
- Then 相同 key 下由显式 middleware 配置优先

### Requirement: Engine supports minimal adaptive download delay

库必须提供最小 `AutoThrottle` 能力，按 origin 基于延迟、错误和目标并发动态调整下载间隔。

#### Scenario: AutoThrottle raises delay after slow or failed requests

- Given 某个 origin 开启了 `auto_throttle`
- When 最近一次请求延迟明显升高、返回 `429 / 5xx`，或下载直接失败
- Then 引擎会提高这个 origin 的后续 delay

#### Scenario: AutoThrottle reduces unnecessary delay after recovery

- Given 某个 origin 开启了 `auto_throttle`
- When 后续请求恢复为较快且成功的响应
- Then 引擎会把这个 origin 的 delay 逐步调回更低值，而不是一直停留在错误退避后的高位

#### Scenario: AutoThrottle gates same-origin requests by target concurrency

- Given 某个 origin 开启了 `auto_throttle`
- And 这个 origin 当前 inflight 请求数已经达到 `target_concurrency`
- When 引擎继续尝试发送同 origin 的新请求
- Then 这些请求会被要求退避，而不是继续立即下载

### Requirement: Engine 提供显式 request dedup 组件

库必须把 request dedup 作为显式 engine 组件装配，而不是继续默认编译成 middleware。

#### Scenario: Default engine uses in-memory dedup

- Given 调用方直接使用 `Engine::new()`
- When 引擎准备把 request 放进 scheduler
- Then 默认 dedup 实现是 `dedup::Memory`

#### Scenario: Built-in Bloom dedup can be selected explicitly

- Given 调用方显式调用 `Engine::with_dedup(dedup::Bloom::default())`
- When 引擎准备把 request 放进 scheduler
- Then 该请求会走布隆过滤器去重
- And 调用方清楚这是一种近似 dedup，存在误判边界

#### Scenario: Callers can replace dedup explicitly

- Given 调用方调用 `Engine::with_dedup(...)`
- When 引擎准备把 request 放进 scheduler
- Then 实际 dedup 行为由这个显式 dedup 组件决定

#### Scenario: Duplicate requests are rejected before scheduler

- Given dedup 组件判定某个 request 已经见过
- When 引擎准备把这个 request 入队
- Then 该 request 在进入 scheduler 前被拒绝

#### Scenario: dont_filter requests bypass dedup

- Given 某个 request 显式设置了 `dont_filter = true`
- When 引擎准备把这个 request 入队
- Then 引擎不会因为 dedup 组件命中而拦截它

### Requirement: Engine 提供显式下载组件装配入口

库必须允许调用方显式替换 HTTP downloader 与 browser downloader，而不需要每次都回退到完整 `from_parts(...)` 构造。

#### Scenario: Default engine uses built-in downloaders

- Given 调用方直接使用 `Engine::new()`
- When 引擎执行普通 HTTP request 与 browser request
- Then 默认分别使用 `download::Http` 与 `download::Browser`

#### Scenario: Callers can replace only the HTTP downloader

- Given 调用方调用 `Engine::with_http(...)`
- When 引擎执行普通 HTTP request
- Then HTTP 下载行为由这个显式 HTTP downloader 决定
- And 现有 browser downloader 保持不变

#### Scenario: Callers can replace only the browser downloader

- Given 调用方调用 `Engine::with_browser(...)`
- When 引擎执行 browser request
- Then browser 下载行为由这个显式 browser downloader 决定
- And 现有 HTTP downloader 保持不变

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

#### Scenario: File store can rotate output into numbered files

- Given 调用方对内置 `store::File` 使用 `with_rotate_items(...)` 或 `with_rotate_bytes(...)`
- When store 按顺序写入多个 item
- Then 输出会按阈值切分到编号文件
- And 默认命名保持在同一个基础路径上追加序号，例如 `items-0001.jsonl`

#### Scenario: File store can switch to a readable pretty block format

- Given 调用方对内置 `store::File` 使用 `with_format(store::FileFormat::PrettyJsonBlocks)`
- When store 写入 item
- Then 它仍然走同一条最终 `store` 边界
- And 每条 item 会以可读的 pretty JSON block 形式落盘

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

#### Scenario: Webhook store retries retryable failures with explicit backoff

- Given 调用方对内置 `store::Webhook` 设置 `with_retry_limit(...)` 与 `with_retry_backoff(...)`
- And 请求错误或目标接口返回 `429 / 5xx`
- When store 写入某个 item
- Then store 会按配置的 backoff 重试
- And 其它非 `2xx` 继续直接返回显式错误

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

#### Scenario: Kafka store can attach message key and headers

- Given 调用方对内置 `store::Kafka` 使用 `with_key(...)`、`with_key_field(...)`、`with_header(...)` 或 `with_header_field(...)`
- When store 写入某个 item
- Then 它在继续发送完整 item JSON value 的同时，也附带对应的 message key 与 headers
- And 如果从 item 字段取值失败，store 返回显式错误

#### Scenario: Custom store implementations plug into the same final item chain

- Given 调用方自己实现了 `store::Store`
- When 它通过 `Engine::with_store(...)` 挂到引擎上
- Then 自定义 store 仍然走同一条 `parse -> item -> pipeline -> store` 主链
- And 如果它覆盖了 `batch_write(...)`，引擎也会优先使用该批量路径

#### Scenario: Built-in store maintenance scope remains explicit

- Given 调用方需要 PostgreSQL、对象存储、复杂第三方 API 或更高阶 MQ 语义
- When 它规划最终输出能力
- Then 框架继续建议通过自定义 `store::Store` 扩展
- And 当前内置维护范围明确保持在 `Memory / File / Sqlite / Webhook / Redis / Kafka`

### Requirement: Engine exposes minimal runtime stats

库必须提供最小运行时计数快照，方便调用方读取核心执行计数。

#### Scenario: Engine reports request response retry error and item counters

- Given 引擎已经执行过请求、重试、item 处理与错误路径
- When 调用方读取 `engine.stats()`
- Then 返回的快照包含 `request_count`、`response_count`、`error_count`、`retry_count`、`item_count` 与 `pipeline_drop_count`
- And 也包含 `dedup_reject_count`、`robots_disallow_count`、`robots_delay_count`、`http_cache_hit_count`、`http_cache_revalidate_count`、`http_cache_store_count`、`http_cache_miss_count` 与 `store_error_count`

#### Scenario: Stats count only items that were written after pipeline

- Given 某个 item 被 pipeline 显式丢弃
- When 调用方读取 `engine.stats()`
- Then `pipeline_drop_count` 增加
- And `item_count` 不增加

#### Scenario: Stats are cumulative for one engine instance

- Given 同一个 engine 实例连续执行了多次任务
- When 调用方读取 `engine.stats()`
- Then 快照中的计数是该 engine 实例生命周期内的累计值

#### Scenario: Engine supports a minimal stats reporter hook

- Given 调用方想把运行时计数变化转发给自定义观测组件
- When 它调用 `Engine::with_stats_reporter(...)`
- Then 引擎继续保留 `engine.stats()` 作为主读取 API
- And 每次累计计数更新时都会把对应 event 与最新 snapshot 推给这个 reporter

### Requirement: Engine supports a minimal robots.txt crawl policy

库必须提供最小 `robots.txt` 抓取策略，并明确默认行为与当前受限边界。

#### Scenario: Engine exposes robots as an explicit component

- Given 调用方想替换默认 robots policy
- When 它调用 `Engine::with_robots(...)`
- Then 引擎改用这个显式 robots 组件判断请求是否允许继续

#### Scenario: robots.txt policy is disabled by default

- Given 调用方没有显式开启 robots 策略
- When 引擎执行请求
- Then 引擎不会额外因为 `robots.txt` 阻止请求

#### Scenario: Engine skips a disallowed request before download

- Given 调用方通过 `Settings::with_robots_obey(true)` 开启 robots 策略
- And 当前 origin 的 `robots.txt` 禁止访问该请求路径
- When 引擎准备下载该请求
- Then 引擎在真正下载前跳过该请求

#### Scenario: Engine delays a request when robots crawl-delay applies

- Given 调用方通过 `Settings::with_robots_obey(true)` 开启 robots 策略
- And 当前 origin 的 `robots.txt` 为匹配到的 user-agent group 声明了 `Crawl-delay`
- When 引擎准备继续下载同一个 origin 的下一个请求
- Then 引擎按该 delay 退避并重试，而不是把请求当成永久拒绝

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

#### Scenario: Robots matching supports wildcard and group specificity

- Given 某个 robots policy 同时声明了多个 `User-agent` group 与带 `*` / `$` 的规则
- When 引擎判断某个请求路径和当前 user-agent
- Then 更具体的 group 优先于 wildcard group
- And 路径匹配支持 `*` wildcard 与末尾 `$` end anchor

#### Scenario: Robots component can expose sitemap URLs

- Given 当前 origin 的 `robots.txt` 声明了一个或多个 `Sitemap`
- When 调用方读取 robots 组件的 sitemap 信息
- Then 组件能够返回这些 sitemap URL

#### Scenario: Engine can turn robots sitemaps into seed requests

- Given 调用方开启 `Settings::with_robots_sitemap_seeds(true)`
- And 当前 origin 的 `robots.txt` 声明了一个或多个 `Sitemap`
- When 引擎启动并处理 start URLs
- Then 引擎会抓取这些 sitemap 文档，并把其中声明的页面 URL 自动加入种子请求集合
- And 这些自动发现的种子请求仍然走引擎现有的 dedup 路径
- And 当前实现保持默认 `priority = 0` 与 `depth = 0`

#### Scenario: Default robots memory policy uses an in-memory cache backend

- Given 调用方使用默认 `robots::Memory`
- When 同一个 origin 的 robots policy 被重复读取
- Then 默认 cache backend 是进程内的 `robots::cache::Memory`

#### Scenario: Callers can replace the robots cache backend

- Given 调用方保留 `robots::Memory` 这套 robots 语义
- And 它通过 `robots::Memory::with_cache(...)` 提供自定义 cache backend
- When robots 组件加载或保存某个 origin 的 policy
- Then 这些缓存读写会走这个显式 cache backend

#### Scenario: Built-in file robots cache persists entries across engine restarts

- Given 调用方使用 `robots::cache::File`
- When 某个 origin 的 robots policy 被保存到这个 backend
- Then 后续新的 engine 实例仍然可以从同一个 cache 文件恢复该 origin 的 robots policy

#### Scenario: Default robots cache uses a TTL-based refresh window

- Given 调用方使用默认 `robots::Memory`
- When 某个 origin 的 robots policy 还在默认 `24h` 的 `cache_ttl` 内
- Then 引擎会继续复用现有缓存，而不会每次都重新抓取 `robots.txt`

#### Scenario: Stale robots cache falls back to the previous policy on refresh failure

- Given 某个 origin 已经有过期的 robots cache 条目
- When 引擎尝试刷新它，但这次抓取 `robots.txt` 失败或返回临时非成功状态
- Then 引擎优先继续复用这条旧 cache policy
- And 不会因为这次刷新失败直接把旧 policy 替换成新的 fail-open 缓存条目

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

#### Scenario: Redis scheduler reclaims stale inflight tasks after lease timeout

- Given 调用方使用内置 `scheduler::Redis`
- And 某个 task 已经进入 `inflight`
- And 原 worker 在 lease timeout 内没有 `complete()` 或 `requeue()`
- When 后续 worker 继续访问同一个 Redis scheduler namespace
- Then 这个 stale `inflight` task 会被回收到 `ready` 或 `delayed`
- And 它继续沿用原始 task identity，而不是生成新的任务身份

#### Scenario: Redis scheduler validates worker ownership before resolving a lease

- Given 调用方使用内置 `scheduler::Redis`
- And 某个 worker 已经 claim 了一条 task 并拿到对应 lease
- When 另一个 worker 试图用不同 worker identity 完成或重排这条 task
- Then scheduler 会拒绝这次 complete 或 requeue
- And 旧 lease 不会覆盖当前 inflight owner

#### Scenario: Engine renews Redis task leases while long-running work is still active

- Given 调用方使用内置 `scheduler::Redis`
- And 它给当前 scheduler 配置了 `lease_timeout` 与 `heartbeat_interval`
- And 某个 task 的实际处理时间长于第一次 lease timeout 窗口
- When engine 仍在处理这条 task
- Then engine 会按 heartbeat interval 续租当前 lease
- And 其它 worker 在这段时间里不会把这条 task 提前回收到 ready 或 delayed

#### Scenario: Redis scheduler snapshot remains an instantaneous namespace view

- Given 调用方使用内置 `scheduler::Redis`
- When 它调用 `scheduler.snapshot().await?`
- Then 它会读到该 namespace 当前这一刻的 `ready / delayed / inflight` 与最小 lease / reclaim 状态
- And 这份结果不等价于 `Engine::stats()` 那种 engine 生命周期累计计数

#### Scenario: Redis durable scheduler can inspect multiple namespaces by prefix

- Given 同一个 Redis 实例里同时跑了多个 durable scheduler namespace
- When 调用方调用 `scheduler::Redis::namespaces_with_prefix(...)` 或 `scheduler::Redis::namespace_snapshots_with_prefix(...)`
- Then 它可以按前缀发现并批量读取这些 namespace 的运维概览
- And 这层能力继续属于 Redis-specific durable scheduler API，而不是共享 `scheduler::Scheduler` trait

#### Scenario: Checkpoint restore remains a snapshot boundary instead of runtime reclaim

- Given 调用方使用 `scheduler::checkpoint::Memory` 从某个 checkpoint 恢复
- And 该 checkpoint 里本来就存在 `inflight` task
- When scheduler 完成恢复
- Then 它恢复的是 checkpoint 当时保存的 `ready / delayed / inflight` 快照
- And 它不会把 checkpoint 误当成带 lease reclaim 的 runtime durable scheduler

#### Scenario: Custom scheduler and checkpoint backends reuse the same runtime boundary

- Given 调用方需要自定义 scheduler 或 checkpoint 持久化后端
- When 它分别实现 `scheduler::Scheduler` 或 `scheduler::checkpoint::Persist`
- Then 引擎仍然复用同一个 task state 与 checkpoint 边界

#### Scenario: Scheduler remains the only task queue boundary

- Given 调用方想替换任务排队或 ready task 选择行为
- When 它扩展引擎的任务调度能力
- Then 它继续实现 `scheduler::Scheduler`
- And 框架不会再额外引入独立 `queue` 组件去分裂同一条任务语义

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

#### Scenario: Minimal HTTP cache adds conditional request headers from cached validators

- Given 调用方通过 `Settings::with_http_cache(true)` 开启最小 HTTP cache
- And 某个 HTTP `GET` 请求之前已经缓存了 `ETag` 或 `Last-Modified`
- When 引擎再次执行同一个请求
- Then downloader 前的 runtime 会自动补 `If-None-Match` 或 `If-Modified-Since`

#### Scenario: Minimal HTTP cache restores cached response on 304

- Given 调用方通过 `Settings::with_http_cache(true)` 开启最小 HTTP cache
- And 某个 HTTP `GET` 请求之前已经缓存了响应 body 与对应 validator
- When 服务端对后续同请求返回 `304 Not Modified`
- Then 引擎会把缓存 body 回填成正常 `Response`
- And `Response.flags` 包含 `http_cache`

#### Scenario: Default HTTP cache uses an in-memory cache backend

- Given 调用方通过 `Settings::with_http_cache(true)` 开启 HTTP cache
- When 它没有显式替换 cache backend
- Then 默认 backend 是进程内 `middleware::http_cache::Memory`

#### Scenario: Callers can persist HTTP cache entries to a file backend

- Given 调用方想跨 engine 实例复用 HTTP cache
- When 它通过 `Settings::with_http_cache_file(...)` 或 `HttpCache::with_cache(...)` 选择 `middleware::http_cache::File`
- Then HTTP cache 条目会持久化到磁盘文件

#### Scenario: HTTP cache entries expire by ttl

- Given 某个 HTTP cache 条目已经超过配置的 `ttl`
- When 引擎再次执行同一个请求
- Then 这条条目不会继续参与条件请求回源
- And 引擎会把它视为 cache miss

#### Scenario: HTTP cache supports validator-only and response strategies

- Given 调用方显式设置了 HTTP cache `strategy`
- When `strategy = validators`
- Then 引擎只缓存 `ETag / Last-Modified`
- And 服务端返回 `304 Not Modified` 时不会回填旧 body
- When `strategy = response`
- Then 引擎还会缓存响应 body
- And 服务端返回 `304 Not Modified` 时会回填旧 body

#### Scenario: HTTP cache stats include miss and store counters

- Given 调用方读取 `engine.stats()`
- When HTTP cache 发生 miss、revalidate、store 或 hit
- Then 快照会累计 `http_cache_miss_count`、`http_cache_revalidate_count`、`http_cache_store_count` 与 `http_cache_hit_count`
