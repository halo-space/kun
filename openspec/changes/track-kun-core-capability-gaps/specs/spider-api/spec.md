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

### Requirement: Follow Request Derivation Must Preserve Shared Request Semantics

系统 MUST 让 `response.follow()` 与显式构造 `Request` 使用同一套请求能力模型。

#### Scenario: Follow request inherits core request properties

- **WHEN** 用户从响应派生 follow request
- **THEN** follow request 的 mode、meta 以及后续定义的 timeout / session / proxy 等核心请求语义遵循统一规则

#### Scenario: Follow request resets per-request payload

- **WHEN** 用户从已有 request 派生新的 follow request
- **THEN** 新请求继承 headers、cookies 与核心请求配置，但默认重置 method 为 `GET`、清空 body、清空 callback，并且不继承 `dont_filter`

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

#### Scenario: Unsupported browser options fail explicitly

- **WHEN** browser request 启用了尚未接线的 `stealth` 或 `fingerprint_profile`
- **THEN** 系统返回显式 download error，而不是静默忽略这些配置

#### Scenario: Browser session can reuse persisted profile state

- **WHEN** 多个 browser request 使用相同的 session id
- **THEN** downloader 复用同一个稳定的 Playwright user data dir
- **AND** cookies 与 local storage 等浏览器态数据可以随 session 继续复用

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

#### Scenario: Response text respects declared charset before UTF-8 fallback

- **WHEN** 响应头或文档声明了 `charset`
- **THEN** `Response.text` 优先按声明编码解码 `Response.body`
- **AND** 当没有可用编码声明时，系统回退为 UTF-8 lossy 解码

#### Scenario: HTML XPath behavior is explicit and testable

- **WHEN** 用户在 HTML 响应上使用 XPath
- **THEN** 框架要么提供稳定支持，要么给出明确、稳定、可测试的限制行为

#### Scenario: OCR-capable selector type is not schema-only

- **WHEN** 规则 schema 或解析能力暴露了 OCR 相关 selector/type
- **THEN** 该能力必须有对应的运行时实现，或从公开 schema 中移除
