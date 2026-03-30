# 规范增量

## ADDED Requirements

### Requirement: Shared Validation Capability

系统必须把字段或 item 的运行时校验能力实现为代码爬虫与 DSL 共享的底层能力，而不是仅作为 rules 配置字段存在。

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

系统必须让 `response.follow()` 与显式构造 `Request` 使用同一套请求能力模型。

#### Scenario: Follow request inherits core request properties

- **WHEN** 用户从响应派生 follow request
- **THEN** follow request 的 mode、meta 以及后续定义的 timeout / session / proxy 等核心请求语义遵循统一规则

#### Scenario: Follow request resets per-request payload

- **WHEN** 用户从已有 request 派生新的 follow request
- **THEN** 新请求继承 headers、cookies 与核心请求配置，但默认重置 method 为 `GET`、清空 body、清空 callback，并且不继承 `dont_filter`

## MODIFIED Requirements

### Requirement: Response 提供内建解析辅助方法

库必须在 `Response` 上提供 CSS、XPath、JSON、XML、Regex、AI 与 Feed 的解析辅助方法，并明确这些能力在 HTML、XML 与扩展场景下的真实边界。

#### Scenario: HTML XPath behavior is explicit and testable

- **WHEN** 用户在 HTML 响应上使用 XPath
- **THEN** 框架要么提供稳定支持，要么给出明确、稳定、可测试的限制行为

#### Scenario: OCR-capable selector type is not schema-only

- **WHEN** 规则 schema 或解析能力暴露了 OCR 相关 selector/type
- **THEN** 该能力必须有对应的运行时实现，或从公开 schema 中移除
