# 第一轮开发清单：阶段 1 与阶段 2
日期：2026-03-10

本文件只覆盖 `IMPLEMENTATION_TASKS.md` 中的：

- 阶段 1：基础对象层
- 阶段 2：解析能力层

目标：

- 先把对外 API 最底层的对象和查询能力做扎实
- 先不要进入 Scheduler、Engine、插件、完整 middleware 细节

---

## A) 阶段 1：基础对象层

### A1. 统一错误类型

输出物：

- `src/error.rs`

第一轮只做：

- `SpiderError` 枚举
- 覆盖这些错误类别：
  - request build
  - download
  - parse
  - rules
  - plugin
  - scheduler
  - engine

完成标准：

- 所有后续模块都统一复用这一错误类型

### A2. Request 对象

输出物：

- `src/request.rs`

第一轮只做：

- `RequestMode`
  - `Http`
  - `Browser`
- `Request`
  - `url`
  - `mode`
  - `method`
  - `headers`
  - `body`
  - `meta`
  - `callback`
  - `dont_filter`
  - `runtime`

必须确认：

- `Request.mode = http | browser`
- 两类 request 对外统一一个对象

完成标准：

- 能构造一个最小 GET request
- 能构造一个 browser-mode request

### A3. Response 对象

输出物：

- `src/response.rs`

第一轮只做：

- `Response`
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

必须确认：

- `response.meta` 来自 request
- `body` 与 `text` 同时存在
- `follow()` 默认继承当前 meta

完成标准：

- 能从一个已有 response 构造 follow request

### A4. Item 最小表示

输出物：

- `src/item.rs`
- `src/item/output.rs`

第一轮只做：

- 最小 `Item`
- 最小 `ItemOutput` trait

完成标准：

- 不引入复杂 item 类型系统

### A5. 本轮自测

至少自测这几个点：

1. `Request::new()` 能构造默认 HTTP request
2. 手动切换到 `RequestMode::Browser` 不报错
3. `Response::default()` 存在完整字段
4. `response.follow("/x")` 会继承当前 meta

---

## B) 阶段 2：解析能力层

### B1. 统一查询对象

输出物：

- `src/parser/query.rs`

第一轮先定住接口骨架，不追求全部实现。

建议先明确：

- 节点查询
- 值查询

最小方法集合：

- `one()`
- `all()`
- `text()`
- `html()`
- `attr(name)`
- `value()`
- `group(index)`

必须确认：

- 文本类默认 trim
- 结构化值不默认 trim
- 后续 `options={"trim": False}` 能有挂点

### B2. CSS 查询

输出物：

- `src/parser/css.rs`

第一轮目标：

- 先定 `CssQuery`
- 先把 `Response.css()` 调用链跑通

完成标准：

- `response.css("h1.title")` 能返回查询对象

### B3. XPath 查询

输出物：

- `src/parser/xpath.rs`

第一轮目标：

- 接口和 CSS 尽量一致

完成标准：

- `response.xpath("//h1")` 能返回查询对象

### B4. JSON 查询

输出物：

- `src/parser/json.rs`

第一轮目标：

- 支持无 selector
- 支持 selector

完成标准：

- `response.json(None)`
- `response.json(Some("$.data.id"))`

两种入口都能跑通

### B5. XML 查询

输出物：

- `src/parser/xml.rs`

第一轮目标：

- 明确 XML 属于 P0
- 先把查询对象和接口钉住

完成标准：

- `response.xml("...")` 能返回查询对象

### B6. Regex 查询

输出物：

- `src/parser/regex.rs`

第一轮目标：

- 支持 pattern
- 支持 `group(index)`
- 预留 trim 关闭点

完成标准：

- `response.regex("...")` 返回查询对象

### B7. AI 查询

输出物：

- `src/parser/ai.rs`

第一轮目标：

- 明确 AI 属于 P0
- 先把 `response.ai(prompt, source="html", options=None)` 接口钉住
- 先不追求完整 provider 实现

完成标准：

- `response.ai("提取标题")` 返回查询对象
- `options` 参数有挂点

### B8. Response 解析入口接线

输出物：

- `src/response.rs`

第一轮要接上的方法：

- `css()`
- `xpath()`
- `json()`
- `xml()`
- `regex()`
- `ai()`

完成标准：

- 这些方法都能从 `Response` 直接调用

### B9. 本轮自测

至少自测这几个点：

1. `response.css("h1")`
2. `response.xpath("//h1")`
3. `response.json(None)`
4. `response.xml("//item")`
5. `response.regex("id=(\\w+)")`
6. `response.ai("提取标题")`

---

## C) 第一轮结束定义

当下面条件都满足时，第一轮可以算完成：

- `Request / Response` 基础结构稳定
- `Request.mode = http | browser` 已进入模型
- `Response.follow()` 可用
- 六类解析入口已经全部接线：
  - css
  - xpath
  - json
  - xml
  - regex
  - ai
- 代码还不需要真正抓网页，也不需要真正执行完整 Engine

第一轮目标是：

> 把对象和解析接口模型稳定下来，为后续 Rules、Middleware、Downloader、Engine 铺路。
