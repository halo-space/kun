# P0 实现任务拆分表
日期：2026-03-10

本文件用于把 `README.md` 中已经确认的设计，拆成真正可执行的 Rust P0 开发顺序。

适用范围：

- Rust 2024
- 不使用 `mod.rs`
- 目标是先跑通最小可用闭环，而不是一次性实现全部扩展能力

P0 的目标不是“功能全面”，而是：

> 跑通一个既支持代码 callback，又支持本地 JSON DSL，同时具备 `runtime`、`MIDDLEWARES`、插件清单、重试/去重/调度基础能力的最小抓取链路。

补充定位：

> 这个仓库是库本身。内置 `rules` / `middleware` 直接在库里提供，不靠库内部 `plugins.toml` 自注册；`plugins.toml` 主要留给使用该库的业务项目声明自定义扩展。

---

## 1) P0 范围边界

### 必须实现

- `Spider`
- 默认代码入口 `parse`
- `Request / Response`
- `response.css / xpath / json / xml / regex / ai / follow`
- `Request.mode = http | browser`
- `rules = { type, options }`
- `rules.type = local / inline`
- DSL 基础结构：`steps / parse.fields / parse.links / runtime`
- `runtime.schedule / retry / dedup`
- `MIDDLEWARES`
- `download / spider` 两类 middleware
- 内建 middleware：`retry_by_status / retry_by_error / dedup / rate_limit / cookies / proxy`
- `plugins.toml + 自动加载`
- 插件类别：`middleware / rules`
- 内存版 `Scheduler`
- `http` 下载器
- `browser` 下载器最小闭环
- `Engine` 主执行链

### 明确不进 P0

- `ocr`
- `provider / storage` 插件类别
- 完整事件系统
- callback 插件化
- 远程规则热更新
- 完全自由的 merge/override 机制

---

## 2) 推荐目录

```text
src/
  lib.rs

  spider.rs
  request.rs
  response.rs

  parser.rs
  parser/
    css.rs
    xpath.rs
    json.rs
    xml.rs
    regex.rs
    ai.rs
    query.rs

  rules.rs
  rules/
    source.rs
    inline.rs
    local.rs
    schema.rs
    validate.rs
    compile.rs

  runtime.rs
  runtime/
    compile.rs

  middleware.rs
  middleware/
    traits.rs
    chain.rs
    types.rs
    retry_by_status.rs
    retry_by_error.rs
    dedup.rs
    rate_limit.rs
    cookies.rs
    proxy.rs

  plugins.rs
  plugins/
    manifest.rs
    loader.rs
    registry.rs
    types.rs

  scheduler.rs
  scheduler/
    traits.rs
    memory.rs
    types.rs

  download.rs
  download/
    traits.rs
    http.rs
    browser.rs

  engine.rs
  engine/
    context.rs
    types.rs

  item.rs
  item/
    output.rs

  error.rs
```

说明：

- 顶层 `foo.rs` 负责导出模块入口
- `foo/` 目录放细分实现
- 遵循 Rust 2024 风格，不使用 `mod.rs`

---

## 3) 开发阶段总览

建议按 8 个阶段推进，每个阶段都要能独立验收。

1. 基础对象层
2. 解析能力层
3. Spider 与 callback 层
4. Rules 加载与 DSL 校验层
5. Middleware 与 runtime 编译层
6. 调度与下载层
7. Engine 主链路层
8. 插件清单与自动加载层

---

## 4) 阶段 1：基础对象层

### 目标

先把最基础的对外对象定住，后面所有模块都依赖它们。

### 模块

- `error.rs`
- `request.rs`
- `response.rs`
- `item.rs`

### 任务

#### 4.1 定义统一错误类型

- 建立最小错误枚举或错误结构
- 区分：
  - request 构造错误
  - 下载错误
  - 解析错误
  - rules 加载错误
  - 插件加载错误
  - 调度错误

#### 4.2 定义 `Request`

字段至少包含：

- `url`
- `mode`
- `method`
- `headers`
- `body`
- `meta`
- `callback`
- `dont_filter`
- `runtime`

并明确 request 统一分为两种模式：

- `mode = http`
- `mode = browser`

两类 request 共用同一个对外对象，底层由不同下载器执行。

#### 4.3 定义 `Response`

字段至少包含：

- `url`
- `status`
- `headers`
- `body`
- `text`
- `meta`
- `request`
- `flags`
- `certificate`
- `ip_address`
- `protocol`

#### 4.4 定义基础 `Item`

- P0 先支持最小 item 表示
- 不做重型 item 类型系统

### 验收标准

- `Request` / `Response` 字段与 `README.md` 对齐
- `Response.meta` 可以从 request 透传
- `Response.text` 与 `body` 可同时存在
- `Request.mode` 已纳入模型，且 `http/browser` 语义明确

---

## 5) 阶段 2：解析能力层

### 目标

先做 P0 的统一解析能力，让代码 `parse` 可以真正拿到值。

### 模块

- `parser.rs`
- `parser/css.rs`
- `parser/xpath.rs`
- `parser/json.rs`
- `parser/xml.rs`
- `parser/regex.rs`
- `parser/ai.rs`
- `parser/query.rs`

### 任务

#### 5.1 设计统一查询结果对象

至少支持：

- `one()`
- `all()`
- `text()`
- `html()`
- `attr(name)`
- `value()`
- `group(index)`

#### 5.2 实现 `css`

- 支持常见 DOM 查询
- 支持 `.text()`
- 支持 `.attr()`
- 默认 trim

#### 5.3 实现 `xpath`

- 支持 XPath 查询
- 接口风格与 css 尽量一致

#### 5.4 实现 `json`

- 支持无参取整个 JSON
- 支持路径查询

#### 5.5 实现 `xml`

- 支持 XML 查询
- 接口风格尽量与 `xpath`/结构化取值保持一致

#### 5.6 实现 `regex`

- 支持 `group(n)`
- 支持 `options.trim`
- 默认文本结果 trim

#### 5.7 实现 `ai`

- 提供统一 `response.ai(prompt, source="html", options=None)` 入口
- 支持最小可用调用链与 `options` 透传
- P0 重点是接口、配置和执行链打通；具体 provider 可先接最小实现或 mock
- 后续真实模型调用优先评估基于 Rust 库 `async-openai` 封装 provider/client，但不影响当前主链路推进

### 验收标准

- `response.css(...).text()`
- `response.xpath(...).text()`
- `response.json(...).value()`
- `response.xml(...).value()` 或等价查询
- `response.regex(...).group(1)`
- `response.ai(...).value()`

都能工作

---

## 6) 阶段 3：Spider 与 callback 层

### 目标

把对外 Spider 体验先跑起来，不依赖 DSL 也能用。

### 模块

- `spider.rs`

### 任务

#### 6.1 定义 Spider 抽象

至少包含：

- `name`
- `start_urls`
- `runtime`
- `MIDDLEWARES`
- `rules`
- 默认 `parse`

#### 6.2 设计 callback 查找机制

- 默认入口永远是 `parse`
- 其他 callback 为显式注册或方法映射
- 不做 step id 到 callback 的自动猜测

#### 6.3 设计 `response.follow()`

至少支持：

- `response.follow(url)`
- `response.follow(url, callback=...)`
- `response.follow(url, callback=..., meta=...)`

### 验收标准

- 纯代码 Spider 能跑一个最简单的：
  - `start_urls -> parse -> follow -> parse_detail`

---

## 7) 阶段 4：Rules 加载与 DSL 校验层

### 目标

让 `rules` 真正能从 `inline/local` 变成标准 DSL。

### 模块

- `rules.rs`
- `rules/source.rs`
- `rules/inline.rs`
- `rules/local.rs`
- `rules/schema.rs`
- `rules/validate.rs`
- `rules/compile.rs`

### 任务

#### 7.1 定义 `rules = { type, options }`

P0 只支持：

- `inline`
- `local`

#### 7.2 定义 P0 DSL schema

顶层只支持：

- `steps`
- 可选 `runtime`
- 可选 `MIDDLEWARES`

说明：

- 顶层不再支持 `entry`
- 默认入口固定为 `parse`
- 若 DSL 接管默认入口，必须存在 `id = "parse"` 的 step

#### 7.3 定义 `step`

P0 step 字段：

- `id`
- `impl`
- `callback`，仅 `impl=code`
- `fetch`
- `parse`
- `route`
- `output`
- `runtime`
- 可选 `MIDDLEWARES`

#### 7.4 校验规则

- `impl=dsl` 不能再写 `callback`
- `impl=code` 必须写 `callback`
- `links[*].to.next_step` 必须指向存在的 step

#### 7.5 compile 到内部定义

- 目标不是直接执行 JSON
- 而是把 DSL 转成统一的内部 step 定义

### 验收标准

- `inline` 和 `local` 都能加载
- 非法 DSL 会在校验阶段直接报错
- 合法 DSL 能被编译成统一内部表示

---

## 8) 阶段 5：Middleware 与 runtime 编译层

### 目标

把高层 `runtime` 和显式 `MIDDLEWARES` 串起来。

### 模块

- `middleware.rs`
- `middleware/traits.rs`
- `middleware/chain.rs`
- `middleware/types.rs`
- `middleware/retry_by_status.rs`
- `middleware/retry_by_error.rs`
- `middleware/dedup.rs`
- `middleware/rate_limit.rs`
- `middleware/cookies.rs`
- `middleware/proxy.rs`
- `runtime.rs`
- `runtime/compile.rs`

### 任务

#### 8.1 定义 middleware trait

固定三类方法：

- `process_request`
- `process_response`
- `process_exception`

#### 8.2 定义 middleware chain

支持：

- `download`
- `spider`

#### 8.3 实现内建 middleware

P0 先做：

- `retry_by_status`
- `retry_by_error`
- `dedup`
- `rate_limit`
- `cookies`
- `proxy`

#### 8.4 runtime 编译

把：

- `retry`
- `dedup`
- `schedule`

编译成默认 middleware 配置。

#### 8.5 合并规则

- `runtime` 生成默认 middleware
- 显式 `MIDDLEWARES` 覆盖同名 key
- `enabled = false` 表示禁用

### 验收标准

- 可以从 Spider `runtime` 编译出默认 middleware
- 可以被显式 `MIDDLEWARES` 覆盖
- request/response/exception 三段链路可执行

---

## 9) 阶段 6：调度与下载层

### 目标

先跑通内存调度 + HTTP 下载。

### 模块

- `scheduler.rs`
- `scheduler/traits.rs`
- `scheduler/memory.rs`
- `scheduler/types.rs`
- `download.rs`
- `download/traits.rs`
- `download/http.rs`
- `download/browser.rs`

### 任务

#### 9.1 定义 Scheduler trait

最小接口：

- `enqueue`
- `lease`
- `ack`
- `nack`

#### 9.2 实现内存版 Scheduler

- 只要求 P0 可用
- 不要求分布式

#### 9.3 定义 Downloader trait

- `fetch(request) -> response`

#### 9.4 实现 HTTP downloader

- 支持最小 GET/POST
- 支持 headers/body
- 支持 timeout

#### 9.5 实现 Browser downloader 最小闭环

- 保留 `Request.mode = browser`
- 支持最小 Playwright 驱动接入
- 保留 `Chromium` / `Google Chrome` engine 配置入口
- 为 stealth / fingerprint profile / challenge-page 处理预留扩展字段
- P0 先追求能跑通最小页面抓取，不追求完整能力覆盖

### 验收标准

- 能把 URL 放进 scheduler
- 能 lease 出来并用 HTTP 下载
- 能得到完整 `Response`
- 能针对 `mode = browser` 跑通一条最小下载链路

---

## 10) 阶段 7：Engine 主链路层

### 目标

把前面所有模块连起来，形成真正的最小执行闭环。

### 模块

- `engine.rs`
- `engine/context.rs`
- `engine/types.rs`

### 任务

#### 10.1 组装执行上下文

至少包含：

- Spider
- Request
- Response
- 当前 runtime
- middleware chain

#### 10.2 串起主链路

执行顺序：

1. `Scheduler.lease`
2. request middleware
3. download
4. response / exception middleware
5. callback 或 DSL step
6. 产出 item / follow request
7. ack / nack / retry

实现约束：

- 像 `execute_spider_once()` 这种命名可以作为过渡实现存在，但后续主循环成型后，必须优先收敛成更清晰的 `run()` 语义，或者直接并入最终 engine 主循环，不能长期保留为最终架构命名。

#### 10.3 统一代码 callback 和 DSL step

- 代码 callback 和 DSL step 最终都走一套 engine 调度

### 验收标准

至少跑通这三条链路：

1. 纯代码 Spider
2. 纯 DSL Spider（`rules.type=local`）
3. 混合 Spider（DSL step 跳到代码 callback）

---

## 11) 阶段 8：插件清单与自动加载层

### 目标

让最终配置用户只配 key，不关心注册细节。

### 模块

- `plugins.rs`
- `plugins/manifest.rs`
- `plugins/loader.rs`
- `plugins/registry.rs`
- `plugins/types.rs`

### 任务

#### 11.1 解析 `plugins.toml`

P0 只支持插件类别：

- `middleware`
- `rules`

#### 11.2 建立 `(kind, name)` 注册表

- 不使用全局单名字空间
- 允许：
  - `middleware.redis`
  - `rules.redis`

#### 11.3 冲突检测

硬规则：

- 同类同名冲突时
- 只有显式 `override = true` 才允许覆盖
- 否则直接报错

#### 11.4 自动加载到系统

- 配置用户不手动注册
- 库内置能力直接可用
- 使用方项目通过自己的 `plugins.toml` 提供额外能力

### 验收标准

- 使用方项目的 `plugins.toml` 可加载
- 自定义 `middleware` 和 `rules` 插件可被发现
- 未注册 key 在加载/校验阶段直接报错

---

## 12) 最小验收场景

P0 完成后，至少要跑通下面三个示例。

### 场景 A：纯代码 Spider

- `start_urls`
- `parse`
- `response.follow`
- `parse_detail`
- `yield item`

### 场景 B：纯本地规则 Spider

- `rules.type = local`
- DSL `id = "parse"` step
- `parse.fields`
- `parse.links`

### 场景 C：混合 Spider

- 默认 DSL step `parse`
- `next_step -> detail`
- `detail.impl = code`
- `callback = parse_detail`

---

## 13) 推荐开发顺序

如果只按一个顺序推进，建议严格按下面来：

1. `error`
2. `request`
3. `response`
4. `parser`
5. `spider`
6. `rules`
7. `middleware`
8. `runtime`
9. `scheduler`
10. `download`
11. `engine`
12. `plugins`

不要先做：

- browser
- AI/OCR
- provider/storage 插件
- 事件系统
- 高级动态配置

补充说明：

- `AI` 解析能力的真实 provider 接入已经预留方向，后续可优先基于 `async-openai` 实现
- 当前阶段仍然先保持 `parser::ai` 为接口和执行链占位，不让模型接入阻塞主流程

---

## 14) 每阶段完成后都要做的事情

每完成一个阶段，都建议做三件事：

1. 写一个最小样例验证当前模块
2. 校验没有和 `README.md` 的设计口径冲突
3. 记录哪些地方属于 P1/P2，避免偷偷扩 scope

---

## 15) 当前最重要的开发纪律

- 默认入口永远是 `parse`
- 顶层 DSL 不再重新引入 `entry`
- `runtime` 和 `MIDDLEWARES` 不要混成一层
- `MIDDLEWARES` 必须固定为 `enabled / type / order / options`
- 浏览器模式设计要预留，但不要提前实现过深
- Rust 2024，不使用 `mod.rs`
- 显式优先于隐式
- 能在校验阶段报错的，绝不留到运行时静默失败
