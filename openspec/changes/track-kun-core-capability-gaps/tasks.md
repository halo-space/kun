# 任务清单

## 1. 共享 Validation 能力

- [x] 1.1 定义共享 `Validation` / `ValidationRule` 结构，并把它从 `rules` 的配置字段提升为可复用的底层能力。
- [x] 1.2 在 DSL 执行链中实际执行字段校验，而不是只解析 `step.validate`。
- [x] 1.3 为代码爬虫提供直接调用同一套 validation 能力的 API，而不是要求代码模式自行手写重复逻辑。
- [x] 1.4 明确校验失败的运行时语义：报错、丢弃还是可配置策略。
- [x] 1.5 同步 `openspec/specs/spider-api/spec.md`、`openspec/specs/rules-dsl/spec.md` 与相关测试。
- [x] 1.6 为共享 `Validation.field` 补最小字段路径能力，至少支持对象路径、数组索引与数组展开的逐值校验。

## 2. Request / Follow / Session 能力

- [x] 2.1 为 `Request` 补齐 timeout、proxy、cookies/session 等请求级能力建模。
- [x] 2.2 重新梳理 `response.follow()` 的派生语义，明确哪些 request 属性应继承、哪些可覆盖。
- [ ] 2.3 让代码爬虫与 DSL 生成的 request 都走同一套 request 能力模型。
- [x] 2.4 同步 `openspec/specs/spider-api/spec.md` 与 request/follow 相关测试。
- [x] 2.5 把 request-level cookies 从 `http` 私有配置进一步收口为真正的共享 request 能力，避免 browser request 使用 cookies 时退回 `Http` 模式。

## 3. Scheduler 与任务身份

- [x] 3.1 为 scheduler 引入稳定的 task identity，而不是只按 URL ack/nack。
- [x] 3.2 明确 retry、delayed task、inflight task 在 task identity 下的行为。
- [x] 3.3 校验 scheduler 在“同 URL 不同 meta/body/method”场景下的正确性。
- [x] 3.4 同步 `openspec/specs/runtime-engine/spec.md` 与 scheduler 测试。
- [x] 3.5 明确当前 scheduler 的 memory-only 边界，并规划 durable scheduler / checkpoint 持久化 的最小接口或实现方向。

## 4. Cookies / Proxy / HTTP 真实能力

- [x] 4.1 将 `cookies` middleware 从空壳实现补成真实行为，并与 request/session 语义对齐。
- [x] 4.2 将 `proxy` middleware 从空壳实现补成真实行为，并明确 rotate/failure/pool 的最小能力边界。
- [x] 4.3 为 HTTP downloader 补 timeout、cookie jar、proxy、redirect 相关能力的统一接线。
- [x] 4.4 同步 `openspec/specs/middleware-plugins/spec.md`、`openspec/specs/runtime-engine/spec.md` 与网络层测试。

## 5. Browser 能力边界

- [x] 5.1 统一 browser request 配置模型与实际 downloader 实现，消除配置语义和实现语义不一致的问题。
- [x] 5.2 明确未启用 browser feature 时的行为边界，应为显式失败还是受限 stub。
- [x] 5.3 为 browser 模式补最小可验证的真实行为测试或契约测试。
- [x] 5.4 同步 `openspec/specs/spider-api/spec.md` 与相关文档。
- [x] 5.5 为 browser response 补真实的 `status`、`headers`、`protocol`、`ip_address`、`certificate` 等网络元数据，或明确并固化受限语义。
- [x] 5.6 让 browser downloader 接住统一 request-level cookies 语义，而不是只复用 session 持久态。
- [x] 5.7 为相同 browser session 的并发执行建立最小协调策略，避免共享 user data dir 时出现竞态。

## 6. Parser 缺口补齐

- [x] 6.1 补齐 HTML XPath 能力，或明确提供稳定的 HTML XPath 替代实现。
  - 当前已基于 `xrust` 接住统一 XPath 执行，并在 HTML 场景先通过 DOM 解析与规范化生成稳定节点树，再提供 `one()`、`all()`、`text()`、`html()` 与 `attr()` 的一致语义。
- [x] 6.2 为 `ocr` selector_type 补真正 parser/runtime 支持，或从 schema 中移除占位能力。
  - 当前已从公开 schema 与编译入口移除 `ocr` selector_type，不再保留未实现的公开占位能力。
- [x] 6.3 规划 parse 后处理能力：多选择器兜底、normalize、类型转换、结构化校验。
  - 当前已补一组更完整的 query transform：`fallback(...)`、`fallback_many(...)`、`field(...)`、`filter_field_present(...)`、`filter_field_equals(...)`、`pick_fields([...])`、`index(...)`、`flatten()`、`compact()`、`trim()`、`first_non_empty()`、`skip(...)`、`take(...)`、`last()`、`dedup()`、`join(...)`、`split(...)`、`replace(...)`、`normalize_whitespace()`、`parse_number()`、`parse_bool()`、`parse_json()`、`parse_datetime()`、`parse_datetime_with_format(...)`。
  - 当前也已补最小 query 级断言：`require_non_empty()`、`require_one()`；结构过滤/投影与空数组空对象的存在性语义也已统一收口。
- [x] 6.4 同步 `README.md`、`TODO.md` 与 parser 测试。

## 7. Pipeline / Store Item 链路

- [x] 7.1 将当前轻量 `Pipeline` 收口为唯一的 item 处理链路，并把最终持久化/投递沉到独立 `Store` 边界。
- [x] 7.2 提供最小内置 store 能力，并让 engine 走 `pipeline -> store` 语义。
- [x] 7.3 明确 pipeline 丢弃、pipeline 错误与 store 错误的运行时语义。
- [x] 7.4 同步 `openspec/specs/runtime-engine/spec.md` 与 pipeline/store 测试。

## 8. Plugin 扩展边界

当前决策：`Engine::load_plugins()` 只支持 `middleware` kind；`rules`、`provider`、`storage` 先保留命名空间，不作为已落地运行时能力对外承诺。

- [x] 8.1 明确 plugin 体系当前支持的 kind 边界，避免 registry 泛化而 engine 只支持部分 kind。
- [x] 8.2 评估是否需要在 middleware 之外扩展 plugin kind，以及扩展前的底层前提条件。
- [x] 8.3 同步 `openspec/specs/middleware-plugins/spec.md` 与插件加载校验测试。

## 9. DSL 与共享底层能力对齐

当前后置：先补齐并稳定代码爬虫与共享底层能力，再回头收口 DSL 配置面与运行时映射。

- [ ] 9.1 逐项梳理 DSL 字段里哪些能力已经进入共享底层，哪些仍然只是配置占位。
- [ ] 9.2 确保 `validate`、request、cookies、proxy、output 等能力优先作为底层能力实现，再映射到 DSL。
- [ ] 9.3 同步 `openspec/specs/rules-dsl/spec.md`，明确 DSL 不应发明独立运行时。

## 10. 文档与持续验证

- [x] 10.1 在 `README.md` 中维护一份简短的“已完成底层能力 / 待补能力”说明。
- [x] 10.2 按能力子项补充单元测试或集成测试，避免只补文档不补验证。
- [x] 10.3 每完成一个能力分组后运行 `cargo test`。
- [x] 10.4 如涉及示例变化，运行 `cargo check --examples`。

## 11. Engine 结构整理

- [x] 11.1 收口 `engine` 中任务执行参数，把 task run / reservation 相关上下文整理成更稳定的结构体，而不是继续扩散参数列表。
- [x] 11.2 评估 `TaskOutcome` 的体积差异，明确是否通过 `Box` 或其它方式降低大枚举变体的拷贝/栈占用。
- [x] 11.3 在不改变现有运行时语义的前提下，为 `engine` 结构整理补最小回归测试。
- [x] 11.4 清理 `engine` 内部局部命名，去掉 `sem` 一类缩写，统一使用 `global_semaphore`、`domain_semaphore` 这类完整命名。
- [x] 11.5 如果本轮仍暂时保留单文件实现，下一轮评估将任务执行相关逻辑拆到独立子模块，至少明确 `task executor` / `task run apply` 的拆分边界。
- [x] 11.6 统一 `engine` 内部任务执行命名语义，避免 `TaskOutcome`、`TaskRun`、`run()`、`map_flow_to_task_outcome()` 这类跨层级混用；明确“middleware flow”“task run/outcome”“run apply/handle”各自的命名边界。

## 12. Response Body / Text 语义

- [x] 12.1 明确 `Response.body` 保存原始字节，`Response.text` 是从 `body` 解码得到的字符串视图，而不是单独维护另一份来源。
- [x] 12.2 为 `Response` 构造路径补统一的文本解码逻辑，优先使用 BOM、`Content-Type charset` 与文档声明，再回退 UTF-8 lossy。
- [x] 12.3 为 HTTP 下载链路补编码回归测试，并同步 `openspec/specs/spider-api/spec.md`、`README.md`、`TODO.md`。

## 13. Browser 渲染下载边界

- [x] 13.1 明确 browser request 保持“渲染型下载器”定位，聚焦导航、等待页面就绪与最终 HTML 获取，不继续公开点击、滚动、脚本执行这类页面动作配置。
- [x] 13.2 明确 browser 与现有 `wait_for`、timeout、session 之间的执行顺序和失败语义。
- [x] 13.3 为 browser 渲染下载边界补单元测试或契约测试，并同步 `README.md`、`docs/capabilities.md` 与 `openspec/specs/spider-api/spec.md`。

## 14. Durable Scheduler 与任务排序

- [x] 14.1 为 `Task` 增加 priority / depth 这类通用调度元数据，并明确 memory scheduler 的取任务顺序。
- [x] 14.2 提供内置 durable scheduler checkpoint 持久化，至少支持基于磁盘文件的快照持久化与恢复。
  - 当前已在磁盘文件之外补内置 `scheduler::checkpoint::Redis`，用于基于 Redis 的 scheduler checkpoint 持久化与恢复；如果调用方需要其它后端，可以自行实现 `scheduler::checkpoint::Persist`。
- [x] 14.3 如果可行，提供内置的 persisted memory scheduler 包装层，让 enqueue / complete / requeue 后自动落盘。
- [x] 14.3.1 当前也已补直接基于 Redis 的 `scheduler::Redis`，用于真正的 durable scheduler 后端，而不只是 checkpoint 持久化；如果调用方需要其它 scheduler 后端，可以自行实现 `scheduler::Scheduler`。
- [x] 14.4 同步 `README.md`、`docs/capabilities.md`、`openspec/specs/runtime-engine/spec.md` 与 scheduler 测试。

## 15. Parser / Validation 语义继续收口

- [x] 15.1 在现有 query transform 基础上补更常用的后处理能力，例如 URL 拼接、日期时间解析、结构化 map/filter 或其它代码爬虫直接需要的转换。
  - 当前已补 `resolve_url(base_url)`，用于把相对链接解析成绝对 URL，并对空串、非字符串与无效 base URL 显式报错。
  - 当前也已补 `parse_datetime()` 与 `parse_datetime_with_format(...)`，用于把常见日期时间文本收口成规范化字符串，并支持显式 `strptime` 格式。
  - 当前还已补 `trim()`、`skip(...)`、`take(...)`、`last()`、`dedup()`、`split(...)` 与 `parse_json()`，用于更常见的结果切片、字符串拆分和嵌入 JSON 提取链路。
- [x] 15.2 为共享 `Validation` 补更完整的规则语义，例如列表/对象级约束、字段转换后的再校验或可配置失败策略。
  - 当前已补显式文本/列表/对象约束：`with_min_length(...)`、`with_max_length(...)`、`with_min_items(...)`、`with_max_items(...)`、`with_min_fields(...)`、`with_max_fields(...)`、`with_required_fields([...])`。
  - 当前也已补 `ValidationTransform` 链式转换：`Trim`、`NormalizeWhitespace`、`ParseNumber`、`ParseBool`、`ParseDatetime`，支持字段值先收口再走最终类型与规则校验。
  - 当前也已补 `with_object_validations([...])`、`with_each_validations([...])` 与 `validate_fields_report()` / `validate_item_report()`，用于嵌套对象/列表成员规则与 collect-all 错误报告。
  - 当前也已补 `with_all_of([...])`、`with_any_of([...])`、`with_one_of([...])`、`with_mutually_exclusive([...])` 与 `Validation::root()`，用于跨字段组合约束与顶层作用域校验。
  - 当前也已补最小条件约束：`with_when_exists(...)`、`with_when_missing(...)`、`with_when_equals(...)`、`with_when_not_equals(...)` 与 `with_required_when_*`，用于按条件启用 optional/required 校验。
- [x] 15.3 同步 `README.md`、`TODO.md`、`docs/capabilities.md` 与 parser / validation 测试。

## 16. 内置 Store 输出与外部投递

- [x] 16.1 在 `parse -> item -> pipeline -> store` 模型上补内置 SQLite store 能力。
- [x] 16.2 明确 PostgreSQL 不再作为内置 store 维护，统一走用户自定义 `Store` 扩展。
- [x] 16.3 明确内置数据库 store 的建表、字段映射、错误语义与最小示例。
  - 当前 SQLite 已明确：自动建表、不自动清空旧数据、每条 item 保留完整 `item_json`、显式字段列类型映射，以及类型不匹配时显式报错。
  - PostgreSQL 这类外部系统当前统一建议通过自定义 `Store` 接入，而不是继续扩展内置分支。
- [x] 16.4 明确 store 作为统一最终输出/投递边界，后续 API 推送、Redis、Kafka 与其它文件输出都继续挂在这条线上，而不是再拆独立 sink runtime。
  - 当前已补最小 `store::Webhook`，用于把完整 item JSON 推送到 HTTP endpoint。
  - 当前也已补内置 `store::Redis`，支持把完整 item JSON 通过 `SADD` 写入目标 set。
  - 当前也已补内置 `store::Kafka`，支持把完整 item JSON 作为消息 value 写入目标 topic。
  - 更多文件 sink 继续沿用同一条 store 扩展边界。
- [x] 16.5 同步 `openspec/specs/runtime-engine/spec.md`、`README.md`、`docs/capabilities.md` 与相关测试。
  - 当前也已明确 `Store::batch_write(...)` 语义：engine 会对同一次输出里保留下来的 items 优先走批量写入；默认实现回退为逐条 `write(...)`，内置 `Memory`、`File`、`Redis`、`Sqlite` 已补原生批量路径。

## 17. Runtime 观测与抓取策略

- [x] 17.1 提供最小 stats / metrics 能力，至少能统计请求、响应、错误、重试、item、pipeline 丢弃等核心计数。
  - 当前已提供 `Engine::stats()`，返回最小 `stats::Snapshot`，包含 `request_count`、`response_count`、`error_count`、`retry_count`、`item_count` 与 `pipeline_drop_count` 六个累计计数。
- [x] 17.2 提供最小 robots.txt 抓取策略能力，并明确默认行为与可配置边界。
  - 当前已提供 `Settings::with_robots_obey(true)` 与 `Settings::with_robots_user_agent(...)`；引擎会按 origin 内存缓存 `robots.txt`，支持 `User-agent` / `Allow` / `Disallow`、`Crawl-delay`、更完整 `group` 匹配、`* / $` wildcard 规则，并在下载前跳过不允许的请求或按 crawl delay 退避；`robots::Robot::sitemaps(...)` 也可读取声明的 sitemap URL。
- [x] 17.3 提供最小 HTTP cache / conditional request 能力，至少支持 ETag / Last-Modified 语义。
  - 当前已补 `http_cache` download middleware；通过 `Settings::with_http_cache(true)` 开启后，HTTP `GET` 请求会基于缓存的 `ETag / Last-Modified` 自动补 `If-None-Match / If-Modified-Since`，并在 `304 Not Modified` 时回填缓存 body。
- [x] 17.4 同步 `README.md`、`docs/capabilities.md`、相关 specs 与测试。

## 18. 显式 Dedup 组件

- [x] 18.1 把 request dedup 从默认 runtime/middleware 接线收口为显式 engine 组件，并提供 `Engine::with_dedup(...)`。
- [x] 18.2 提供最小内置 dedup 能力与自定义扩展边界，至少包含 `dedup::Memory`、`dedup::Noop` 与 `dedup::Dedup` trait。
- [x] 18.3 同步 `README.md`、`docs/capabilities.md`、`openspec/specs/runtime-engine/spec.md` 与相关测试。

## 19. 显式 Robots 组件

- [x] 19.1 把内部 robots policy 提升成公开组件边界，并提供 `Engine::with_robots(...)`。
- [x] 19.2 提供最小内置 robots 能力与自定义扩展边界，至少包含 `robots::Memory`、`robots::Noop` 与 `robots::Robot` trait。
- [x] 19.3 同步 `README.md`、`docs/capabilities.md`、`openspec/specs/runtime-engine/spec.md` 与相关测试。

## 20. 显式下载组件

- [x] 20.1 为引擎补齐显式 `Engine::with_http(...)` 与 `Engine::with_browser(...)`，让调用方可以单独替换 HTTP 或 browser 下载器。
- [x] 20.2 明确默认下载器仍是 `download::Http` 与 `download::Browser`，并补最小测试覆盖“单独替换其中一个下载器”的行为。
- [x] 20.3 同步 `README.md`、`docs/capabilities.md` 与 `openspec/specs/runtime-engine/spec.md`。

## 21. 对齐 Scrapy 剩余核心缺口

- [x] 21.1 为 `Request` 补齐 `errback` 与 `kwargs` 这类回调失败路由和回调上下文能力，并明确 `follow()` / 子请求的继承与重置语义。
- [ ] 21.2 `signals / extensions` 当前后置，不作为这一轮底层能力实现范围。
- [x] 21.3 提供最小自适应限速能力，对齐 Scrapy `AutoThrottle` 的核心场景，至少明确基于延迟、错误和并发反馈的动态 download delay 语义。
- [x] 21.4 在最小 robots 基础上继续补更完整语义，至少评估 `Crawl-delay`、`Sitemap` 与更完整 wildcard / group 匹配规则。

## 下一轮重点 TODO（非 DSL 主线）

当前建议优先顺序：

- `22 -> 25 -> 23 -> 24 -> 28 -> 26 -> 27 -> 29`
- `6.1 / 6.2 / 9.x / 21.2` 继续按既定决策后置，不纳入这一轮主线实现

## 22. P0 Robots 持久化与自动种子

- [x] 22.1 把 `robots` 当前的 origin 级内存 cache 收口成可替换边界，至少明确持久化 cache 的最小接口与默认行为。
- [x] 22.2 提供内置持久化 `robots` cache 实现，至少覆盖磁盘文件或 Redis 其中一种稳定方案。
- [x] 22.3 让 `robots::Robot::sitemaps(...)` 能按可配置策略自动转成 engine 种子请求，并明确与 `dedup`、`depth`、`priority` 的协同语义。
  - 当前通过 `Settings::with_robots_sitemap_seeds(true)` 显式开启；引擎会抓取 robots 声明的 sitemap / sitemapindex，并把页面 URL 继续走同一条 `enqueue_request(...)` + dedup 路径；当前默认 `priority / depth` 保持 `0`。
- [x] 22.4 明确 `robots` cache 的过期、刷新与抓取失败回退语义，避免长期复用陈旧 policy。
  - 当前默认按 `24h` 的 `cache_ttl` 复用 robots policy；条目过期后会尝试刷新，刷新失败时优先回退旧缓存；如果调用方需要不同语义，可以通过 `with_cache_ttl(...)` 覆盖或 `without_cache_ttl()` 关闭自动过期。
- [x] 22.5 同步 `README.md`、`docs/capabilities.md`、相关 specs 与测试。
- [x] 22.6 为 `robots::Memory` 补最小站点策略开关，允许调用方在 `robots.txt` 临时不可用且无可用缓存时，显式选择 fail-open 或 fail-closed。
  - 当前已补 `robots::UnavailablePolicy::{AllowAll, DisallowAll}` 与 `robots::Memory::with_unavailable_policy(...)`；默认保持原来的 fail-open，调用方也可以显式切到更保守的 `DisallowAll`，且 stale cache 刷新失败时仍优先复用旧 policy。
- [x] 22.7 为 robots 补 `Request-rate` 运行时语义，并明确与 `Crawl-delay` 的协同规则。
  - 当前已支持解析 `Request-rate: requests / window`，并把它按 `window / requests` 的均匀间隔最小 delay 接入运行时；如果同一个 group 同时声明 `Crawl-delay` 与 `Request-rate`，则取更严格的 delay。
- [x] 22.8 为 robots 临时不可用场景补最小 retry delay 退避，避免同一 origin 每个请求都重复抓取 `robots.txt`。
  - 当前 `robots::Memory` 默认对 temporary unavailable 结果使用 `60s` 的 `unavailable_retry_delay`；有 stale cache 时会优先复用 stale cache，没有可用 cache 时则在这个窗口内复用当前 unavailable policy。调用方也可以通过 `with_unavailable_retry_delay(...)` 覆盖，或通过 `without_unavailable_retry_delay()` 关闭。
- [x] 22.9 为 robots sitemap 自动种子补显式 `priority / depth` 控制，避免固定写死成 `0 / 0`。
  - 当前 `Settings` 已补 `with_robots_sitemap_seed_priority(...)` 与 `with_robots_sitemap_seed_depth(...)`；默认仍保持 `0 / 0`，只有显式配置时才覆盖自动种子请求的调度元数据。

## 23. P0 Dedup 能力继续增强

- [x] 23.1 提供内置 `dedup::Bloom`，明确误判边界、容量参数、哈希策略与默认配置。
  - 当前已补 `dedup::Bloom`；默认参数是 `expected_items = 100_000`、`false_positive_rate = 0.01`，哈希次数和 bitset 大小按常见 Bloom 公式自动推导；它是近似 dedup，存在误判导致的潜在漏抓边界。
- [x] 23.2 明确 `Engine::new()` 默认 dedup 策略是否从精确 `dedup::Memory` 切换到 `dedup::Bloom`；无论最终决策如何，都要把默认语义写清楚。
  - 当前决策是不切换；`Engine::new()` 继续默认使用精确 `dedup::Memory`，`dedup::Bloom` 保持显式 opt-in。
- [x] 23.3 为持久化或远程 dedup 预留稳定扩展边界，保证调用方可以继续实现自定义 `Dedup`。
  - 当前继续沿用统一 `dedup::Dedup` trait 作为稳定扩展边界；内置 `Memory / Bloom / Noop` 都只是这一条边界上的实现。
- [x] 23.4 提供最小自定义 dedup 示例，并同步 `README.md`、`docs/capabilities.md` 与测试。
  - 当前已补 `examples/custom_dedup.rs`，演示用户自定义 `dedup::Dedup` 并通过 `.with_dedup(...)` 接入引擎。

## 24. P0 Runtime Stats 细粒度观测

- [x] 24.1 在当前 `stats::Snapshot` 基础上补更细粒度计数，至少覆盖 `dedup_reject`、`robots_disallow`、`robots_delay`、`http_cache_hit`、`http_cache_revalidate`、`store_error`。
  - 当前 `stats::Snapshot` 已补 `dedup_reject_count`、`robots_disallow_count`、`robots_delay_count`、`http_cache_hit_count`、`http_cache_revalidate_count` 与 `store_error_count`，并接到 dedup、robots、http cache 与 store 写入路径。
- [x] 24.2 明确 runtime 观测边界：是继续扩展 `Snapshot`，还是额外抽出可插拔 reporter / exporter 接口。
  - 当前决策是 `Engine::stats()` / `stats::Snapshot` 继续作为主读取 API；另补最小 `stats::Reporter` 扩展点，但不引入完整 metrics backend。
- [x] 24.3 为后续 Prometheus / OpenTelemetry 这类 exporter 预留不破坏当前 API 的最小接线点，但本轮不要求直接实现完整 exporter。
  - 当前已提供 `Engine::with_stats_reporter(...)`，reporter 会在计数更新时收到 event + snapshot，后续 exporter 可以基于这条边界扩展。
- [x] 24.4 同步 `README.md`、`docs/capabilities.md`、相关 specs 与测试。

## 25. P0 Browser Runtime 收口与增强

- [x] 25.1 把 browser user data dir、临时目录、会话锁这类实现路径里的明显同步 I/O 收口到更适合异步 runtime 的实现方式。
  - 当前 `user data dir`、临时 profile 目录与 session lock 这条路径已经改成 `tokio::fs` + async lock 的实现方式；临时目录也改成显式异步清理，不再依赖 `Drop + std::fs`。
- [x] 25.2 扩展内置 `fingerprint_profile` 集合，并明确每个 profile 对 `user_agent`、`locale`、`timezone`、`languages`、`platform` 等字段的稳定映射。
  - 当前内置 profile 已扩展到 `desktop_zh_cn`、`desktop_en_us`、`desktop_en_gb`、`desktop_ja_jp`、`desktop_de_de`、`desktop_fr_fr`，并在文档里写清稳定映射。
- [x] 25.3 在不突破“渲染型下载器”定位的前提下，补一版更完整的 `stealth` bootstrap。
  - 当前 `stealth` 已补 `navigator.language(s)`、`platform`、`vendor`、`hardwareConcurrency`、`deviceMemory`、`maxTouchPoints`、`plugins`、`mimeTypes`、`pdfViewerEnabled`、screen depth、permissions 查询补丁，以及 Chromium 路线的最小 `window.chrome` / `userAgentData`。
- [x] 25.4 明确自定义 `fingerprint_profile` 的最终策略：支持结构化自定义配置，或继续显式不支持并固化文档边界。
  - 当前决策是继续显式不支持自定义 `fingerprint_profile`；只保留内置名字，未知 profile 直接报错，结构化自定义 profile 暂不纳入这轮。
- [x] 25.5 同步 `README.md`、`docs/capabilities.md`、相关 specs 与测试。

## 26. P1 Durable Scheduler 高阶恢复语义

- [x] 26.1 明确 durable scheduler 对 crash restart、inflight reclaim、stale task 恢复的共享语义。
  - 当前约定是：`scheduler::Redis` 作为 runtime owner，负责 crash restart 后的 stale `inflight` reclaim；回收时任务继续沿用原始 `TaskId`，并重新回到 `ready / delayed` 共享语义。
- [x] 26.2 为 `scheduler::Redis` 补最小 lease / heartbeat 或超时回收策略，避免 worker 崩溃后 inflight task 永久悬挂。
  - 当前已为 `scheduler::Redis` 补最小 `lease_timeout` 语义；默认会给 `inflight` task 建 lease，超时后在后续访问同 namespace 时自动回收；调用方也可以通过 `with_lease_timeout(...)` 覆盖，或 `without_lease_timeout()` 关闭。
  - 当前也已经显式补齐 `worker_id` ownership、lease 校验与 `heartbeat_interval` 续租；engine 运行长任务时会按配置继续 heartbeat，避免任务还在执行却被别的 worker 提前 reclaim。
- [x] 26.3 继续收口 `checkpoint` 与 durable scheduler 的职责边界，避免“状态快照恢复”和“真实运行时调度”语义混在一起。
  - 当前边界是：`checkpoint` 只恢复保存时那份 `ready / delayed / inflight` 快照，不承担 runtime reclaim；真正的 lease reclaim 属于 `scheduler::Redis` 这类 durable scheduler 自身的责任。
- [x] 26.4 同步 `README.md`、`docs/capabilities.md`、相关 specs 与测试。

## 27. P1 Store 输出能力继续扩展

- [x] 27.1 在统一 `store` 边界上补更丰富的文件输出能力，例如 rotate、buffered batch、可配置序列化格式或其它稳定文件 store 形态。
  - 当前 `store::File` 已补 `with_rotate_items(...)`、`with_rotate_bytes(...)` 与 `FileFormat::PrettyJsonBlocks`；默认仍保持 JSON Lines，rotate 后的文件按同一基础路径编号输出。
- [x] 27.2 补更完整的外部投递语义，例如 Kafka message key / headers、Webhook retry / backoff 边界或其它明确的最小扩展点。
  - 当前 `store::Webhook` 已补显式 `retry_limit / retry_backoff`；`store::Kafka` 已补 `with_key(...)`、`with_key_field(...)`、`with_header(...)` 与 `with_header_field(...)`。
- [x] 27.3 明确哪些输出能力继续内置维护，哪些统一通过用户自定义 `Store` 扩展。
  - 当前决策是继续内置维护 `Memory / File / Sqlite / Webhook / Redis / Kafka`；更专门的数据库、对象存储、第三方 API 与复杂 MQ 语义继续建议走自定义 `Store`。
- [x] 27.4 同步 `README.md`、`docs/capabilities.md`、相关 specs 与测试。

## 28. P1 HTTP Cache 能力继续增强

- [x] 28.1 把当前最小内存 `http_cache` 收口成可替换边界，明确 cache backend、key 语义与默认行为。
  - 当前 `middleware::HttpCache` 已收口为公开 `Cache` 边界；默认 backend 是 `middleware::http_cache::Memory`，key 语义是规范化后的完整 URL，包含 `request.http.query`。
- [x] 28.2 为 `http_cache` 提供持久化或跨进程可复用实现，至少覆盖磁盘文件或 Redis 其中一种稳定方案。
  - 当前已提供内置 `middleware::http_cache::File`，默认路径是 `output/http-cache.json`；调用方可以通过 `Settings::with_http_cache_file(...)` 或 `HttpCache::with_cache(...)` 接入。
- [x] 28.3 为缓存条目补 TTL、失效、刷新与条件请求回源策略，避免无限期复用旧 body/validator。
  - 当前默认按 `24h` 的 `ttl` 复用条目；过期条目会转成 miss；命中 `304` 时会刷新条目时间戳，避免无限期复用陈旧 validator。
- [x] 28.4 评估并明确缓存策略分层，至少区分“只做 conditional request validator cache”和“真正 response body cache”两种语义。
  - 当前已提供 `Strategy::Validators` 与 `Strategy::Response` 两种策略，前者只缓存 validator，后者还会缓存响应 body 并在 `304` 时回填。
- [x] 28.5 把 `http_cache_hit`、`http_cache_revalidate`、`http_cache_store`、`http_cache_miss` 等观测点接入统一 stats 语义。
  - 当前已补 `http_cache_hit_count`、`http_cache_revalidate_count`、`http_cache_store_count` 与 `http_cache_miss_count`。
- [x] 28.6 同步 `README.md`、`docs/capabilities.md`、相关 specs 与测试。

## 29. P2 Plugin Kind 扩展边界

- [x] 29.1 明确 `middleware` 之外哪些 plugin kind 真正值得提升为 engine 级装配点，例如 `store`、`scheduler`、`dedup`、`robots` 或其它更稳定 owner。
- [x] 29.2 在不破坏当前 API 的前提下，为候选 plugin kind 设计最小注册与装配边界，避免 manifest 命名空间继续先于底层 owner 扩张。
- [x] 29.3 如果当前阶段仍不扩 plugin kind，也要把“不支持”的边界写进文档和 spec，避免占位命名空间被误解为已落地能力。
- [x] 29.4 同步 `README.md`、`docs/capabilities.md`、相关 specs 与测试。
