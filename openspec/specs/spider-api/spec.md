# Spider API 规范

## 目标

定义 `halo-spider` 对外最核心的 Spider、Request、Response 与解析辅助 API，确保代码模式与 DSL 模式共享同一套运行语义。

### Requirement: Spider 暴露单一默认 parse 入口

库必须以 `Spider` 作为主要对外抽象，并将 `parse` 作为唯一默认回调入口。

#### Scenario: start_urls 默认进入 parse

- Given spider 实现了 `start_urls()`
- When 引擎把这些 URL 入队
- Then 每个生成的请求都通过 `parse` 进入 spider

#### Scenario: 未知回调名称显式报错

- Given 某个请求指向 spider 未实现的回调名
- When `Spider::call()` 执行分发
- Then 框架返回 engine error，而不是静默忽略该回调

### Requirement: Spider 可以同时组合代码回调与 DSL step

库必须允许 spider 在声明 `rules()` 的同时，仍在同一个实例上使用 Rust 回调方法。

#### Scenario: 默认路由由 DSL parse step 处理

- Given 编译后的 rules 中存在 `id = "parse"` 且未声明 `callback` 的 step
- When spider 分发一个缺少 `next_step` 的响应
- Then 该响应由 DSL step 处理

#### Scenario: 代码 step 路由到具名回调

- Given 编译后的 rules 中存在声明了回调名称的 step
- When 响应 metadata 指向该 step
- Then spider 将其分发到对应的 Rust 回调

### Requirement: DSL-first spider 可以在没有自定义分发胶水代码的情况下声明 rules

库必须支持这类 spider：它们依赖 `rules()` 与引擎托管的分发路径，而不需要在 `parse()` 内手动加载、编译或执行 DSL rules。

#### Scenario: 纯 DSL spider 使用默认 parse 路由

- Given spider 提供了 `start_urls()` 与 `rules()`
- And 编译后的 rules 中存在 `id = "parse"` 的 DSL step
- When `Engine::run()` 执行该 spider
- Then 响应由 DSL step 处理，不需要自定义 `parse()` 胶水代码

#### Scenario: DSL spider 仍可与代码回调共存

- Given spider 提供了 `rules()`
- And 部分 step 未声明 `callback`，另一些 step 声明了具名 `callback`
- When 引擎分发响应
- Then DSL step 走 rules 引擎，代码 step 走同一 spider 上的具名回调

### Requirement: Spider 与 DSL 共享运行时 validation 能力

库必须让代码爬虫与 DSL `step.validate` 复用同一套字段或 item 校验能力，而不是分别维护两套语义。

#### Scenario: Code spider can validate parsed data with shared API

- Given 代码爬虫在解析后拿到了字段 map 或 item
- When 它调用共享 validation API
- Then 校验语义与 DSL `step.validate` 保持一致

#### Scenario: Validation failure is reported explicitly

- Given DSL step 或代码爬虫触发了共享 validation
- When 某个字段缺失或类型不匹配导致校验失败
- Then 系统返回 parse error，而不是静默丢弃当前输出

### Requirement: Request 支持 HTTP 与 browser 执行模式

库必须通过统一的 `Request` 类型描述外发工作，并支持 `mode = http | browser`。

#### Scenario: browser follow 请求保留父级模式

- Given 一个 browser 模式的响应
- When `response.follow()` 创建子请求
- Then 子请求仍保持 browser 模式

#### Scenario: 回调与 metadata 随请求传递

- Given 请求中包含 `callback` 和 `meta`
- When 该请求被执行并转换成响应
- Then 响应保留原始 metadata 与回调路由上下文

### Requirement: Request 建模核心请求级能力

库必须在统一的 `Request` 类型上建模 timeout、proxy、session 以及 request 级 cookies/follow 继承语义，而不是把这些能力散落成互不对齐的入口。

#### Scenario: Request carries timeout proxy and session settings

- Given 用户显式构造一个请求
- When 用户设置 timeout、proxy 或 session
- Then 这些值存储在 request 上，供后续 follow、middleware 或 downloader 复用

#### Scenario: Follow inherits core request settings but resets payload

- Given 一个父请求已经声明 headers、cookies、timeout、proxy、session 等核心请求设置
- When 用户调用 `response.follow()` 创建子请求
- Then 子请求继承这些核心设置
- And 子请求默认使用 `GET`
- And 子请求不继承 body、callback 与 `dont_filter`

### Requirement: Response 暴露核心网络与解析状态

库必须在 `Response` 上暴露 `url`、`status`、`headers`、`body`、`text`、`meta`、`request`、`flags`、`certificate`、`ip_address` 与 `protocol`。

#### Scenario: 从 request 构造的 Response 继承 metadata

- Given 一个请求携带 metadata
- When 调用 `Response::from_request()`
- Then 响应 metadata 与原始请求 metadata 一致

#### Scenario: follow 合并 metadata patch

- Given 响应本身已经包含 metadata
- When 提供后续的 metadata patch
- Then 生成的请求保留原有 metadata，并覆盖 patch 中声明的值

### Requirement: Response 提供内建解析辅助方法

库必须在 `Response` 上提供 CSS、XPath、JSON、XML、Regex、AI 与 Feed 的解析辅助方法。

#### Scenario: 查询辅助方法基于 response text 工作

- Given 一个已经解码出文本内容的响应
- When 用户调用 `css`、`xpath`、`json`、`xml`、`regex`、`ai` 或 `feed`
- Then 这些辅助方法都以响应 text 作为输入源
