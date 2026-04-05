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

#### Scenario: Optional validation only applies when configured and present

- Given 代码爬虫或 DSL 只为部分字段声明了共享 validation
- When 某个未声明 validation 的字段不存在，或某条非 `required` 的 validation 对应字段不存在
- Then 系统跳过该字段的 validation
- And 不会因为缺失字段去触发额外的类型、长度或枚举错误

#### Scenario: Conditional validation can gate required and optional rules

- Given 代码爬虫或 DSL 为共享 validation 声明了条件约束
- When 它使用 `with_when_exists(...)`、`with_when_equals(...)` 或 `with_required_when_equals(...)` 这类条件规则
- Then 系统只在条件命中时执行对应 validation
- And 条件未命中时，该 validation 视为 skipped，而不是自动通过或自动报错

#### Scenario: Validation supports explicit text list and object constraints

- Given 代码爬虫或 DSL 需要校验文本长度、列表长度或对象字段数量
- When 它使用 `with_min_length(...)`、`with_max_items(...)`、`with_required_fields([...])` 这类共享 validation 规则
- Then 系统按对应值类型执行显式约束
- And 规则名与实际校验语义保持一致，而不是全部混用为泛化的 `min/max`

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

#### Scenario: Request kwargs remain a dedicated callback context channel

- Given 请求显式声明了 `kwargs`
- When 该请求被执行并转换成响应
- Then 响应可以通过统一入口读取这些 `kwargs`
- And `kwargs` 与 `meta` 保持为两条不同语义的上下文通道

#### Scenario: Request errback handles download or callback failures explicitly

- Given 请求显式声明了 `errback`
- When 下载失败或 spider callback 返回错误
- Then 引擎会把失败上下文分发到对应 errback
- And errback 返回的 `items` 与 `requests` 继续走同一条 engine 主链

#### Scenario: browser 请求走 Playwright 兼容引擎配置

- Given 一个 browser 请求声明 `driver = playwright`
- And `engine` 为 `chromium`、`firefox` 或 `webkit`
- When browser downloader 执行该请求
- Then 它使用对应的 Playwright 浏览器引擎
- And `headless`、`viewport`、`wait_for`、timeout、headers 与 proxy 等已接线配置会被应用

#### Scenario: browser 请求沿用统一 Request 的 method 与 body

- Given 一个 browser 请求设置了非 `GET` method 或 request body
- When browser downloader 执行该请求
- Then downloader 会把这些值覆写到首个目标主文档导航请求
- And 最终仍返回渲染后的页面响应

#### Scenario: 未启用 browser feature 时显式失败

- Given 一个 browser 请求
- When 当前构建未启用 `browser` feature
- Then 执行返回显式 download error
- And 框架不能返回受限 stub response 冒充成功执行

#### Scenario: browser 请求支持内置 fingerprint profile 与最小 stealth bootstrap

- Given 一个 browser 请求启用了内置 `fingerprint_profile` 或 `stealth = true`
- When browser downloader 执行它
- Then downloader 应用稳定的 profile 映射，覆盖 `user_agent`、`locale`、`timezone`、`languages`、`platform`
- And `stealth` 只补最小但可验证的 navigator / window bootstrap，不把 browser 路线扩成通用自动化 runtime

#### Scenario: 未知 browser fingerprint profile 显式报错

- Given 一个 browser 请求设置了未知的 `fingerprint_profile`
- When browser downloader 尝试执行它
- Then 执行返回显式 download error
- And 框架不能静默忽略这个未支持的 profile

#### Scenario: Browser session reuses persisted profile state

- Given 两个 browser 请求声明了相同的 session id
- When browser downloader 执行这些请求
- Then 它们复用同一个稳定的 Playwright user data dir
- And cookies 与 local storage 等浏览器态数据可以随 session id 继续复用

#### Scenario: Browser request uses shared request cookies

- Given 一个 browser 请求在统一 `Request` 上声明了 cookies
- When browser downloader 执行该请求
- Then downloader 把这些 cookies 注入 Playwright browser context
- And 请求不会因为设置 cookies 而退回 `Http` 模式

#### Scenario: Browser session execution is serialized per session id

- Given 两个 browser 请求声明了相同的 session id
- When 引擎并发执行它们
- Then browser downloader 至少按 session id 串行化实际浏览器执行
- And 不同 session id 之间仍可继续并发

#### Scenario: Browser runtime prepares session and temporary directories in an async-friendly way

- Given browser downloader 需要准备 session user data dir 或临时 profile 目录
- When 它进入实际执行路径
- Then 目录准备和清理采用更适合 async runtime 的方式
- And 框架不把明显同步文件 I/O 留在这条高频路径

#### Scenario: Browser route remains a rendering downloader

- Given 一个 browser 请求用于抓取动态页面内容
- When browser downloader 执行该请求
- Then 它聚焦于导航、等待页面就绪和返回最终 HTML
- And 框架不把点击、滚动、脚本执行这类页面动作作为公开共享 request 配置继续扩展

### Requirement: Request 建模核心请求级能力

库必须在统一的 `Request` 类型上建模 timeout、proxy、session 以及 request 级 cookies/follow 继承语义，而不是把这些能力散落成互不对齐的入口。

#### Scenario: Request carries timeout proxy and session settings

- Given 用户显式构造一个请求
- When 用户设置 timeout、proxy 或 session
- Then 这些值存储在 request 上，供后续 follow、middleware 或 downloader 复用

#### Scenario: Request cookies are shared core request state

- Given 用户显式构造一个 HTTP 或 browser 请求
- When 用户设置 request cookies
- Then cookies 存储在统一的 `Request` 上
- And cookies 不会强制切换请求 mode

#### Scenario: Follow inherits core request settings but resets payload

- Given 一个父请求已经声明 headers、cookies、timeout、proxy、session 等核心请求设置
- When 用户调用 `response.follow()` 创建子请求
- Then 子请求继承这些核心设置
- And 子请求默认使用 `GET`
- And 子请求不继承 body、callback、errback、kwargs 与 `dont_filter`

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

#### Scenario: Browser response uses real navigation status and headers

- Given 一个 browser 请求导航到了能返回主文档响应的页面
- When browser downloader 构造 `Response`
- Then `Response.status` 与 `Response.headers` 反映 Playwright 导航响应
- And `Response.flags` 包含 `browser`

#### Scenario: Browser response keeps unavailable network metadata explicit

- Given Playwright 当前没有暴露浏览器导航响应的 `ip_address` 或证书详情
- When browser downloader 构造 `Response`
- Then `Response.ip_address` 与 `Response.certificate` 保持为空
- And `Response.protocol` 继续表示 browser 执行语义，而不是伪造 HTTP 版本值

#### Scenario: Response text is decoded from response body

- Given downloader 返回原始响应字节并构造 `Response`
- When 框架生成 `Response.text`
- Then `Response.body` 保留原始字节
- And `Response.text` 由 `Response.body` 解码得到，而不是来自独立来源

#### Scenario: Response text respects declared charset before apparent encoding and UTF-8 fallback

- Given 响应头或文档声明了 `charset`
- When 框架从 `Response.body` 派生 `Response.text`
- Then 它优先使用声明编码进行解码
- And 如果没有可用编码声明，则先尝试 apparent encoding 猜测
- And 当无法依赖声明编码时，最终仍可回退为 UTF-8 lossy 解码

### Requirement: Response 提供内建解析辅助方法

库必须在 `Response` 上提供 CSS、XPath、JSON、XML、Regex、AI 与 Feed 的解析辅助方法。

#### Scenario: 查询辅助方法基于 response text 工作

- Given 一个已经解码出文本内容的响应
- When 用户调用 `css`、`xpath`、`json`、`xml`、`regex`、`ai` 或 `feed`
- Then 这些辅助方法都以响应 text 作为输入源

#### Scenario: HTML XPath uses a normalized HTML tree

- Given 一个 HTML 响应文本
- When 用户调用 `response.xpath(...)`
- Then 框架先把 HTML 解析并规范化成稳定节点树后再执行 XPath
- And `text()`、`html()` 与 `attr()` 在 HTML / XML 场景下保持一致的最小语义

#### Scenario: Query transforms can resolve relative URLs against a base URL

- Given 用户从响应里提取到相对链接文本
- When 它对 `ValueQuery` 调用 `resolve_url(base_url)`
- Then 系统把相对 URL 解析成绝对 URL
- And 如果输入是空字符串、非字符串或 base URL 无效，则显式返回 parse error

#### Scenario: Query transforms can parse embedded JSON text into structured values

- Given 用户从页面脚本或属性里提取到 JSON 文本
- When 它对 `ValueQuery` 调用 `parse_json()`
- Then 系统把 JSON 文本解析成结构化值
- And 用户可以继续对结果调用 `field(...)`、`index(...)` 等链式读取方法

#### Scenario: Query transforms can slice split and deduplicate result lists

- Given 用户从响应里提取到多值结果或分隔字符串
- When 它对 `ValueQuery` 调用 `skip(...)`、`take(...)`、`last()`、`split(...)` 或 `dedup()`
- Then 系统按声明顺序处理结果集
- And 不会隐式改变其它未声明的 query 语义

#### Scenario: Query transforms can filter and project structured values

- Given 用户从响应里提取到结构化对象结果或对象数组
- When 它对 `ValueQuery` 调用 `filter_field_present(...)`、`filter_field_equals(...)` 或 `pick_fields([...])`
- Then 系统会在同一条 query transform 链路里完成结构过滤或字段投影
- And 用户仍可继续对结果调用 `field(...)`、`index(...)` 等结构化读取方法
