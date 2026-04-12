# 规范增量

## ADDED Requirements

### Requirement: Shared Validation Capability

系统 MUST 把字段或 item 的运行时校验能力实现为代码爬虫与 DSL 共享的底层能力，而不是仅作为 rules 配置字段存在。

#### Scenario: Code spider uses shared validation capability

- **WHEN** 代码爬虫在解析后对字段或 item 发起校验
- **THEN** 它使用与 DSL `step.validate` 同一套底层校验语义

#### Scenario: DSL validation is not configuration-only

- **WHEN** DSL step 声明了 `validate`
- **THEN** 引擎在运行时实际执行校验，而不是仅通过 schema 解析保留该字段

#### Scenario: Validation failure is an explicit parse error

- **WHEN** DSL step 的运行时校验失败，或代码爬虫直接调用共享 validation API 校验失败
- **THEN** 系统返回显式 parse error，而不是静默丢弃 item 或 request

#### Scenario: Optional validations are skipped unless they are explicitly required

- **WHEN** 代码爬虫或 DSL 只为部分字段声明共享 validation，且某条非 `required` 的 validation 对应字段不存在
- **THEN** 系统跳过该字段的 validation，而不是继续执行类型、长度或枚举规则
- **AND** 组合约束中的可选字段不会因为被跳过而自动视为通过

#### Scenario: Conditional validations gate runtime checks explicitly

- **WHEN** 代码爬虫或 DSL 对共享 `Validation` 使用 `with_when_exists(...)`、`with_when_equals(...)` 或 `with_required_when_*` 这类条件规则
- **THEN** 系统只在条件命中时执行对应 validation
- **AND** 条件未命中时，该 validation 视为 skipped，而不是自动通过或自动失败

#### Scenario: Shared validation supports field paths

- **WHEN** 代码爬虫或 DSL 对 `Validation.field` 使用 `meta.title`、`authors[0].name`、`tags[]`、`articles[].title` 这类字段路径
- **THEN** 系统按对象路径、数组索引或数组展开解析目标值
- **AND** 对数组展开路径按每个解析值逐个执行同一套 validation 规则
- **AND** 校验失败时优先返回具体路径，例如 `articles[1].title`

#### Scenario: Shared validation exposes explicit text list and object rules

- **WHEN** 代码爬虫或 DSL 对共享 `Validation` 使用 `with_min_length(...)`、`with_max_items(...)`、`with_required_fields([...])` 这类规则
- **THEN** 系统按文本、列表、对象的对应语义执行校验
- **AND** 规则名与实际行为保持一致，而不是继续全部复用为泛化的 `min/max`

### Requirement: Spider Start Requests Can Use Shared Request Semantics Directly

系统 MUST 允许 spider 从入口就返回完整 `Request`，而不是只能先返回 URL 再由引擎补默认请求。

#### Scenario: Spider can override build_start_requests with full Request values

- **GIVEN** spider 显式覆写了 `build_start_requests()`
- **WHEN** 引擎准备初始化起始请求
- **THEN** 引擎优先使用这些完整 `Request`
- **AND** 这些起始请求可以携带 cookies、proxy、session、headers 或 browser mode 等共享请求能力

### Requirement: Request Exposes Typed Middleware Overrides

系统 MUST 允许代码爬虫把 `dedup / download-before middleware / retry middleware` 作为 typed middleware override 挂在单条 `Request` 上，而不是只能依赖 engine 全局设置。

#### Scenario: Request-local middleware overrides use builder-style APIs

- **WHEN** 调用方给单条 request 配置 `dedup / download-before middleware / retry middleware`
- **THEN** 这些能力优先通过 `Request::with_dedup(...)`、`with_concurrency(...)`、`with_interval(...)`、`with_rate_limit(...)`、`with_auto_throttle(...)`、`with_retry_by_status(...)`、`with_retry_by_error(...)` 这类 builder 暴露
- **AND** 它们与 `with_proxy(...)`、`with_cookie(...)`、`with_timeout(...)` 这类已有 request builder 保持一致的链式使用体验

#### Scenario: Code spider can attach middleware override to one request only

- **WHEN** spider 在同一个 callback 里构造多条 request，但只给其中一条 request 声明 middleware override
- **THEN** 只有这条 request 会使用对应的 dedup、download-before middleware 或 retry middleware 覆盖
- **AND** 其它 request 继续只使用 step 默认值或 engine 全局默认值

#### Scenario: Start requests and follow requests use the same typed middleware model

- **WHEN** spider 通过 `build_start_requests()` 构造 request，或通过 `response.follow()` 派生 request
- **THEN** 它们都使用同一套 typed middleware override capability
- **AND** 框架不会再为“起始 request”和“后续 request”发明两套不同中间件配置模型

#### Scenario: Request can skip middleware by stable exported names

- **WHEN** 调用方需要让当前 request 跳过若干内置 middleware
- **THEN** 它可以通过统一的 `.skip([...])` 入口声明这些 middleware 名字
- **AND** 内置 middleware 会导出稳定常量名，避免用户手写字符串

#### Scenario: Request uses one whole override per middleware

- **WHEN** 调用方多次覆盖同一个 middleware 的配置
- **THEN** 系统按最后一次显式写入的整项配置生效
- **AND** 不对同名 middleware 做字段级深度合并

### Requirement: Request Remains The Primary Spider-Facing Object

系统 MUST 让 `Request` 继续作为代码爬虫里的第一等请求对象，而不是把 callback、meta、runtime 或输出语义分散到 `Response`、`Output` 或 engine 隐式全局上。

#### Scenario: Request carries callback context and middleware overrides together

- **WHEN** spider 构造一条新的 request
- **THEN** callback、errback、meta、cb_kwargs、priority 与 middleware override 都挂在这条 request 上
- **AND** request middleware override 跟着这条 request 进入 scheduler、retry 与后续 follow 链路

#### Scenario: Absolute URL requests do not need Response to be constructed

- **WHEN** spider 已经拿到一条完整绝对 URL
- **THEN** 它可以直接从 `Request::new(url)` 或同类 builder 起手构造下一条 request
- **AND** 不要求调用方为了构造 request 额外依赖 `Response`

### Requirement: Response Provides Navigation Helpers But Does Not Own Middleware Overrides

系统 MUST 把 `Response` 维持为下载结果对象，并只为“基于当前响应方便地产生下一条 request”提供 helper，而不是让 request middleware override 错挂在 `Response` 上。

#### Scenario: Response exposes request shortcuts

- **WHEN** spider 处理某条 response
- **THEN** 它可以通过 `response.meta` 与 `response.cb_kwargs` 读取触发这次下载的 request 上下文
- **AND** 这些 shortcut 与 `response.request` 保持一致的来源语义

#### Scenario: follow only helps derive next requests

- **WHEN** spider 使用 `response.follow(...)` 或 `response.follow_all(...)`
- **THEN** 这些 API 只负责基于当前 response.url 解析相对链接并构造 request
- **AND** request middleware override 仍然需要挂在新 request 自己身上
- **AND** follow API 不是 spider 输出发射接口

### Requirement: Request Bypass Flags Are Explicit And Split

系统 MUST 把“跳过去重”和“跳过域名过滤”拆成显式、独立的 request 语义，而不是继续复用一个 `dont_filter` 字段。

#### Scenario: Skipping dedup does not bypass domain filtering

- **WHEN** 某条 request 显式要求跳过 request dedup
- **THEN** 它只绕过 dedup admission
- **AND** allowed-domain 检查仍然继续执行

#### Scenario: Skipping domain filtering does not bypass dedup

- **WHEN** 某条 request 显式要求跳过 domain filtering
- **THEN** 它只绕过 allowed-domain 检查
- **AND** request dedup 仍然按照 effective policy 继续执行

### Requirement: Follow Request Derivation Must Preserve Shared Request Semantics

系统 MUST 让 `response.follow()` 与显式构造 `Request` 使用同一套请求能力模型，同时明确 request-local middleware override 的默认继承边界。

#### Scenario: Follow request inherits core request properties

- **WHEN** 用户从响应派生 follow request
- **THEN** follow request 的 mode、meta 以及 timeout / session / proxy 等核心请求语义遵循统一规则

#### Scenario: Follow request resets request-local middleware overrides and bypass flags

- **WHEN** 用户从已有 request 派生新的 follow request
- **THEN** 新请求默认不继承父 request 的本地 middleware override
- **AND** 也不继承父 request 的 dedup bypass 或 domain-filter bypass 标记
- **AND** 如果调用方需要这些语义，必须显式重新声明

### Requirement: Request Exposes Errback And Kwargs As Shared Core Semantics

系统 MUST 把 `errback` 与 `kwargs` 收口为统一 `Request` 能力，而不是继续要求调用方退回 `meta` 或私有胶水逻辑。

#### Scenario: Response can read request kwargs through a dedicated entry

- **WHEN** 请求显式声明了 `kwargs` 并被执行成响应
- **THEN** 代码回调可以通过统一响应入口读取这些 `kwargs`
- **AND** `kwargs` 与 `meta` 继续保持不同语义

#### Scenario: Request errback handles download failures

- **WHEN** 请求显式声明了 `errback` 且下载失败
- **THEN** 引擎把失败上下文分发到该 errback
- **AND** errback 返回的输出继续走同一条 engine 主链

#### Scenario: Request errback handles callback failures

- **WHEN** 请求显式声明了 `errback` 且 spider callback 返回错误
- **THEN** 引擎把失败上下文分发到该 errback
- **AND** 失败上下文可以继续读取原请求、原响应与显式 `kwargs`

### Requirement: Browser Execution Must Match Playwright Runtime Boundaries

系统 MUST 让 browser request 的配置语义与实际 downloader 实现保持一致，并把未实现能力收敛为显式失败。

#### Scenario: Browser request uses Playwright-compatible engines

- **WHEN** browser request 选择 `driver = playwright` 与 `chromium | firefox | webkit`
- **THEN** downloader 使用对应的 Playwright 浏览器引擎执行请求

#### Scenario: Browser feature disabled returns explicit error

- **WHEN** 当前构建未启用 `browser` feature 却执行 browser request
- **THEN** 系统返回显式 download error，而不是返回 stub response

#### Scenario: Browser request forwards method and body through the initial navigation request

- **WHEN** browser request 设置了非 `GET` method 或 request body
- **THEN** downloader 在首个目标主文档请求上把这些值覆写到 Playwright 导航请求
- **AND** 后续仍返回渲染后的最终页面 HTML 响应

#### Scenario: Browser request applies built-in fingerprint profiles and minimal stealth bootstrap

- **WHEN** browser request 启用了内置 `fingerprint_profile` 或 `stealth = true`
- **THEN** downloader 应用稳定的内置 profile 映射，覆盖 `user_agent`、`locale`、`timezone`、`languages`、`platform`
- **AND** `stealth` 只补最小但可验证的 navigator / window bootstrap，而不是把 browser 路线扩成通用自动化 runtime

#### Scenario: Unsupported custom fingerprint profile fails explicitly

- **WHEN** browser request 设置了未知的 `fingerprint_profile`
- **THEN** 系统返回显式 download error，而不是静默忽略该配置

#### Scenario: Browser session can reuse persisted profile state

- **WHEN** 多个 browser request 使用相同的 session id
- **THEN** downloader 复用同一个稳定的 Playwright user data dir
- **AND** cookies 与 local storage 等浏览器态数据可以随 session 继续复用

#### Scenario: Browser request consumes shared request cookies

- **WHEN** browser request 在统一 `Request` 上声明了 cookies
- **THEN** downloader 把这些 cookies 注入 Playwright browser context
- **AND** 请求不会因为设置 cookies 而退回 `Http` 模式

#### Scenario: Browser session execution is coordinated per session id

- **WHEN** 多个 browser request 使用相同的 session id 并发执行
- **THEN** downloader 至少按 session id 串行化实际浏览器执行
- **AND** 避免共享 user data dir 时出现竞态

#### Scenario: Browser runtime uses async-friendly session and temporary directory handling

- **WHEN** browser downloader 需要准备 session user data dir 或临时 profile 目录
- **THEN** 它使用更适合 async runtime 的目录准备与清理方式
- **AND** 不再把明显同步文件 I/O 留在这条高频执行路径里

## MODIFIED Requirements

### Requirement: Middleware Override Resolution Is Layered And Non-Inheriting

系统 MUST 让 middleware 覆盖按 `request > current step > engine global` 解析，并禁止 step 之间互相继承。

#### Scenario: Step without explicit middleware uses engine global defaults

- **WHEN** engine 已注册某个 middleware 的全局默认行为，而当前 step 没有显式覆盖该 middleware
- **THEN** 当前 step 下的 request 使用这份 engine 全局默认行为

#### Scenario: One step override does not leak into another step

- **WHEN** `step1` 显式覆盖了某个 middleware，而 `step2` 没有配置该 middleware
- **THEN** `step2` 不继承 `step1` 的覆盖
- **AND** `step2` 只回退到 engine 全局默认行为或空行为

### Requirement: Response 提供内建解析辅助方法

库 MUST 在 `Response` 上提供 CSS、XPath、JSON、XML、Regex、AI 与 Feed 的解析辅助方法，并明确这些能力在 HTML、XML 与扩展场景下的真实边界。

#### Scenario: Browser response uses real navigation status and headers

- **WHEN** browser downloader 导航到了能返回主文档响应的页面
- **THEN** 构造出的 `Response.status` 与 `Response.headers` 反映 Playwright 导航响应
- **AND** `Response.flags` 保留 `browser`

#### Scenario: Browser response keeps unavailable network metadata explicit

- **WHEN** Playwright 当前没有暴露浏览器导航响应的 `ip_address` 或证书详情
- **THEN** 构造出的 `Response.ip_address` 与 `Response.certificate` 保持为空
- **AND** `Response.protocol` 继续表示 browser 执行语义，而不是伪造 HTTP 版本值

#### Scenario: Response text is decoded from response body

- **WHEN** downloader 返回原始响应字节并构造 `Response`
- **THEN** `Response.body` 保留原始字节
- **AND** `Response.text` 由 `Response.body` 解码得到，而不是维护独立来源

#### Scenario: Response text respects declared charset before apparent encoding and UTF-8 fallback

- **WHEN** 响应头或文档声明了 `charset`
- **THEN** `Response.text` 优先按声明编码解码 `Response.body`
- **AND** 当没有可用编码声明时，系统先尝试 apparent encoding 猜测
- **AND** 最终仍可回退为 UTF-8 lossy 解码

#### Scenario: HTML XPath evaluates on a normalized HTML tree

- **WHEN** 用户在 HTML 响应上使用 XPath
- **THEN** 框架先把 HTML 解析并规范化成稳定节点树后再执行 XPath
- **AND** `one()`、`all()`、`text()`、`html()` 与 `attr()` 在 HTML / XML 场景下保持一致的最小语义
- **AND** 常见的非严格 HTML 仍能得到稳定、可测试的提取结果

#### Scenario: Query transforms can resolve relative URLs against a base URL

- **WHEN** 用户对 `ValueQuery` 调用 `resolve_url(base_url)`
- **THEN** 系统把相对 URL 解析成绝对 URL
- **AND** 如果输入是空字符串、非字符串或 base URL 无效，则显式返回 parse error

#### Scenario: Query transforms can parse embedded JSON text into structured values

- **WHEN** 用户对脚本内容或其它 JSON 文本结果调用 `parse_json()`
- **THEN** 系统把文本解析成结构化值
- **AND** 后续可以继续链式调用 `field(...)`、`index(...)` 等结构化读取方法

#### Scenario: Query transforms can slice split and deduplicate result lists

- **WHEN** 用户对 `ValueQuery` 调用 `skip(...)`、`take(...)`、`last()`、`split(...)` 或 `dedup()`
- **THEN** 系统按声明顺序处理结果集
- **AND** 这些后处理仍属于同一条共享 parser 能力链路

#### Scenario: Query transforms can filter and project structured values

- **WHEN** 用户对结构化 `ValueQuery` 调用 `filter_field_present(...)`、`filter_field_equals(...)` 或 `pick_fields([...])`
- **THEN** 系统会在同一条共享 parser 能力链路里完成结构过滤或字段投影
- **AND** 过滤或投影后的结果仍可继续链式调用 `field(...)`、`index(...)` 等结构化读取方法
