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

### Requirement: Follow Request Derivation Must Preserve Shared Request Semantics

系统 MUST 让 `response.follow()` 与显式构造 `Request` 使用同一套请求能力模型。

#### Scenario: Follow request inherits core request properties

- **WHEN** 用户从响应派生 follow request
- **THEN** follow request 的 mode、meta 以及后续定义的 timeout / session / proxy 等核心请求语义遵循统一规则

#### Scenario: Follow request resets per-request payload

- **WHEN** 用户从已有 request 派生新的 follow request
- **THEN** 新请求继承 headers、cookies 与核心请求配置，但默认重置 method 为 `GET`、清空 body、清空 callback / errback / kwargs，并且不继承 `dont_filter`

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

#### Scenario: HTML XPath behavior is explicit and testable

- **WHEN** 用户在 HTML 响应上使用 XPath
- **THEN** 框架要么提供稳定支持，要么给出明确、稳定、可测试的限制行为

#### Scenario: OCR-capable selector type is not schema-only

- **WHEN** 规则 schema 或解析能力暴露了 OCR 相关 selector/type
- **THEN** 该能力必须有对应的运行时实现，或从公开 schema 中移除

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
