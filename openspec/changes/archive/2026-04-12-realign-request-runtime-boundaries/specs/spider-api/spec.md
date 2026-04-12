# 规范增量

## ADDED Requirements

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

## MODIFIED Requirements

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

### Requirement: Middleware Override Resolution Is Layered And Non-Inheriting

系统 MUST 让 middleware 覆盖按 `request > current step > engine global` 解析，并禁止 step 之间互相继承。

#### Scenario: Step without explicit middleware uses engine global defaults

- **WHEN** engine 已注册某个 middleware 的全局默认行为，而当前 step 没有显式覆盖该 middleware
- **THEN** 当前 step 下的 request 使用这份 engine 全局默认行为

#### Scenario: One step override does not leak into another step

- **WHEN** `step1` 显式覆盖了某个 middleware，而 `step2` 没有配置该 middleware
- **THEN** `step2` 不继承 `step1` 的覆盖
- **AND** `step2` 只回退到 engine 全局默认行为或空行为
