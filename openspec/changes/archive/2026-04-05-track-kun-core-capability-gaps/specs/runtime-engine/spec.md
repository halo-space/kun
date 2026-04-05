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

#### Scenario: Checkpoint restore does not pretend to be runtime reclaim

- **WHEN** 调用方通过 `scheduler::checkpoint::Memory` 从 checkpoint 恢复状态
- **AND** checkpoint 里本来就保存了 `inflight` task
- **THEN** 恢复结果仍然只是当时那份 `ready / delayed / inflight` 快照
- **AND** checkpoint 不会额外承担 durable scheduler 的 lease reclaim 语义

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

### Requirement: Request Dedup Is An Explicit Engine Component

系统 MUST 把 request dedup 收口为显式 engine 组件，而不是继续默认依赖 dedup middleware。

#### Scenario: Default engine uses in-memory dedup

- **WHEN** 调用方直接使用 `Engine::new()`
- **THEN** 引擎默认使用 `dedup::Memory`
- **AND** request 会在进入 scheduler 前先经过这个 dedup 组件

#### Scenario: Built-in Bloom dedup can be selected explicitly

- **WHEN** 调用方显式调用 `Engine::with_dedup(dedup::Bloom::default())`
- **THEN** 该请求会走布隆过滤器去重
- **AND** 调用方清楚这是一种近似 dedup，存在误判边界

#### Scenario: Callers can replace dedup explicitly

- **WHEN** 调用方调用 `Engine::with_dedup(...)`
- **THEN** 引擎改用这个显式 dedup 组件决定请求是否可以入队

#### Scenario: Duplicate requests are dropped before scheduler

- **WHEN** dedup 组件判定某个 request 已重复
- **THEN** 引擎不会把该 request 放进 scheduler
- **AND** `dont_filter` request 继续绕过这层 dedup 判断

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

### Requirement: Engine Supports Minimal AutoThrottle

系统 MUST 提供最小 `AutoThrottle` 能力，作为现有下载链路上的自适应限速 middleware，而不是单独发明另一套 runtime。

#### Scenario: Settings derive auto throttle runtime config

- **WHEN** 调用方在 `Settings` 上开启 `with_auto_throttle(true)`
- **AND** 同时设置 `download_delay`、`with_auto_throttle_target_concurrency(...)` 与 `with_auto_throttle_max_delay(...)`
- **THEN** 引擎会把这些值归一化成 `auto_throttle` 所需的 runtime schedule
- **AND** `download_delay` 作为起始/最小 delay 使用，而不是继续单独派生成固定 `interval_gate`

#### Scenario: AutoThrottle raises delay from latency and failures

- **WHEN** 同一个 origin 最近请求变慢、返回 `429 / 5xx`，或下载直接失败
- **THEN** `auto_throttle` 会提高该 origin 的后续 delay

#### Scenario: AutoThrottle respects target concurrency per origin

- **WHEN** 某个 origin 的 inflight 请求数已经达到 `target_concurrency`
- **THEN** 后续同 origin 请求会先退避
- **AND** 其它 origin 的请求仍可继续独立放行

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
- **AND** `Settings::with_robots_obey(...)` 与 `Settings::with_robots_user_agent(...)` 继续只负责启用开关与 user-agent 选择

#### Scenario: robots.txt policy stays disabled unless enabled explicitly

- **WHEN** 调用方未显式开启 robots 策略
- **THEN** 引擎不会额外因为 `robots.txt` 拦截请求

#### Scenario: Disallowed requests are skipped before download

- **WHEN** 调用方通过 `Settings::with_robots_obey(true)` 开启 robots 策略
- **AND** 当前 origin 的 `robots.txt` 不允许该请求路径
- **THEN** 引擎在真正下载前跳过该请求

#### Scenario: Crawl-delay is enforced as a real runtime delay

- **WHEN** 调用方通过 `Settings::with_robots_obey(true)` 开启 robots 策略
- **AND** 当前 origin 的 `robots.txt` 为匹配到的 user-agent group 声明了 `Crawl-delay`
- **THEN** 引擎会按该 delay 退避并重试同 origin 的后续请求
- **AND** 不会把这类请求误当成永久 `Disallow`

#### Scenario: Request-rate is enforced as a real runtime delay

- **WHEN** 调用方通过 `Settings::with_robots_obey(true)` 开启 robots 策略
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

- **WHEN** 调用方开启 `Settings::with_robots_sitemap_seeds(true)`
- **AND** 当前 origin 的 `robots.txt` 声明了一个或多个 `Sitemap`
- **THEN** 引擎会抓取这些 sitemap 文档，并把其中声明的页面 URL 自动加入种子请求集合
- **AND** 这些自动发现的种子请求仍然走引擎现有的 dedup 路径
- **AND** 当前实现保持默认 `priority = 0` 与 `depth = 0`

#### Scenario: Engine can turn gzipped robots sitemaps into seed requests

- **WHEN** 当前 origin 的 `robots.txt` 声明的 sitemap 是常见的 `.xml.gz` 压缩文档
- **THEN** 引擎仍然可以解析它并把里面的页面 URL 自动加入种子请求集合

#### Scenario: Engine can override robots sitemap seed priority and depth

- **WHEN** 调用方开启 `Settings::with_robots_sitemap_seeds(true)`
- **AND** 它额外配置了 `with_robots_sitemap_seed_priority(...)` 或 `with_robots_sitemap_seed_depth(...)`
- **THEN** 引擎生成的 robots sitemap 种子请求会带上这些显式 `priority` / `depth`

#### Scenario: Robots sitemap requests inherit shared request semantics from start requests

- **WHEN** spider 通过 `build_start_requests()` 提供了带 cookies、proxy、session 或 browser mode 的起始请求
- **AND** 调用方开启 `Settings::with_robots_sitemap_seeds(true)`
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

#### Scenario: Callers can overlay explicit per-origin site policy on robots memory

- **WHEN** 调用方保留内置 `robots::Memory`
- **AND** 它通过 `robots::Memory::with_site_policy(...)` 为某个 origin 配置 `robots::SitePolicy`
- **THEN** 这条显式站点策略会叠加在原始 `robots.txt` 语义之上

#### Scenario: Site policy can force access or stricter delay

- **WHEN** 某个 origin 同时存在 `robots.txt` 规则和显式 `robots::SitePolicy`
- **AND** 该站点策略配置了 `SiteAccess::AllowAll`、`SiteAccess::DisallowAll` 或额外 delay
- **THEN** 调用方可以显式覆盖该 origin 的允许/拒绝语义
- **AND** 最终 delay 取 robots delay 和站点策略 delay 里更严格的那个

#### Scenario: Site policy can add sitemap and override unavailable handling per origin

- **WHEN** 调用方对某个 origin 配置了额外 sitemap 或单独的 unavailable policy
- **THEN** 额外 sitemap 会并入当前 origin 的 sitemap 集合
- **AND** 当该 origin 的 `robots.txt` 临时不可用时，优先使用这个 origin 的站点策略 unavailable 处理

### Requirement: HTTP Downloader Wires Shared Transport Semantics

系统 MUST 在 HTTP downloader 中把 timeout、cookie jar、proxy 与 redirect 统一接到共享 request 语义上。

#### Scenario: Session requests share cookie jar state

- **WHEN** 多个请求复用同一个 session 标识
- **THEN** 前一跳响应写入的 cookie 会进入同一 cookie jar，并作用到后续请求

#### Scenario: Request timeout and proxy settings are executed by downloader

- **WHEN** HTTP request 显式声明 timeout、proxy 或 redirect 行为
- **THEN** downloader 依据这些 request 语义执行真实网络行为，而不是忽略或分散到不一致实现中

#### Scenario: Minimal HTTP cache adds conditional request headers from cached validators

- **WHEN** 调用方通过 `Settings::with_http_cache(true)` 开启最小 HTTP cache
- **AND** 某个 HTTP `GET` 请求之前已经缓存了 `ETag` 或 `Last-Modified`
- **THEN** 后续同请求会自动补 `If-None-Match` 或 `If-Modified-Since`

#### Scenario: Minimal HTTP cache restores cached response on 304

- **WHEN** 调用方通过 `Settings::with_http_cache(true)` 开启最小 HTTP cache
- **AND** 某个 HTTP `GET` 请求之前已经缓存了响应 body 与对应 validator
- **AND** 服务端对后续同请求返回 `304 Not Modified`
- **THEN** 引擎会回填缓存响应 body
- **AND** `Response.flags` 会包含 `http_cache`

#### Scenario: Default HTTP cache backend stays in memory unless replaced

- **WHEN** 调用方开启 HTTP cache 但没有显式替换 backend
- **THEN** 默认使用进程内 `middleware::http_cache::Memory`

#### Scenario: HTTP cache can persist entries through a file backend

- **WHEN** 调用方通过 `Settings::with_http_cache_file(...)` 或 `HttpCache::with_cache(...)` 选择 `middleware::http_cache::File`
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
