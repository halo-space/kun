# 爬虫库最终设计稿：Spider、Rules、Runtime、Middlewares
日期：2026-03-10

本文件描述当前确认后的最终推荐设计。目标是做一个真正可复用的爬虫库，同时支持：

- 代码方式编写抓取逻辑
- JSON DSL 规则方式编写抓取逻辑
- 两者在同一 Spider 中混合使用
- 对外使用体验尽量接近 Scrapy
- 对内仍然保持统一的执行模型

本设计稿优先描述对外使用形态；像 `Crawler / Registry / TaskConfig` 这类内容，只作为概念层或内部实现层理解，不默认作为用户主 API。

---

## 1) 核心设计结论

### 1.1 对外主入口

对外主入口统一为 `Spider`。

- 默认代码入口只保留一个：`parse`
- 其他方法全部由用户自定义 callback
- `start_urls` 默认进入 `parse`
- `response.follow(url)` 默认回到 `parse`
- `response.follow(url, callback=self.parse_detail)` 显式跳转到用户自定义 callback

### 1.2 双模式共存

同一个 Spider 可以同时包含：

- 代码 callback
- DSL rules/steps

推荐使用方式：

- 简单、稳定、结构化页面：优先 DSL
- 登录、验证码、签名、复杂详情页：优先代码 callback

### 1.3 `runtime` 的定位

配置里的 `runtime` 表示 Spider 或 step 的执行策略配置块，不等同于内部运行时上下文对象。

`runtime` 主要承载：

- `schedule`
- `retry`
- `dedup`
- middleware 相关高层策略

### 1.4 `MIDDLEWARES` 的定位

`MIDDLEWARES` 是显式 middleware 配置集合；`runtime` 是高层执行策略配置，两者最终会合并成实际执行链。

### 1.5 `rules` 的定位

Spider 上只保留一个规则入口：

- `rules`

`rules` 不是规则内容本身，而是规则来源描述对象。框架会把它加载并归一化为一份标准 JSON DSL。

---

## 2) Spider 对外形态

### 2.1 最小示例

```python
class QuotesSpider(Spider):
    name = "quotes"
    start_urls = ["https://quotes.toscrape.com/"]

    runtime = {
        "schedule": {
            "concurrency": 10
        }
    }

    MIDDLEWARES = {
        "retry_by_status": {
            "enabled": True,
            "type": "download",
            "order": 200,
            "options": {
                "status": [429, 500, 502, 503, 504],
                "backoff_ms": [1000, 3000, 10000]
            }
        }
    }

    rules = {
        "type": "local",
        "options": {
            "path": "rules/quotes.json"
        }
    }

    async def parse(self, response):
        for href in response.css(".quote a::attr(href)").all():
            yield response.follow(href, callback=self.parse_detail)

    async def parse_detail(self, response):
        yield {
            "title": response.css("h1.title").text(),
            "article_id": response.regex(r"article_id=(\\w+)").group(1),
        }
```

### 2.2 Spider 建议包含的字段

- `name`
- `start_urls`
- `runtime`
- `MIDDLEWARES`
- `rules`
- `parse`
- 用户自定义 callback 方法

### 2.3 说明

- `parse` 是默认入口
- 其他 callback 名称完全由用户自定义
- `rules` 可为空，表示纯代码 Spider
- `rules` 可存在，表示纯规则或混合模式 Spider

---

## 3) Request / Response 对外 API

### 3.1 Response 核心属性

建议对外 `Response` 至少提供这些只读属性：

- `url`
- `status`
- `headers`
- `text`
- `body`
- `meta`
- `request`
- `flags`
- `certificate`
- `ip_address`
- `protocol`

建议字段语义如下：

- `url (str)`：当前 response 的最终 URL
- `status (int)`：HTTP 状态码，默认可视为 `200`
- `headers (dict)`：响应头；单值头可为字符串，多值头可为字符串列表
- `body (bytes)`：原始响应字节
- `text (str)`：按编码解码后的文本视图
- `meta (dict)`：由触发该 response 的 request 透传下来的上下文信息
- `request (Request)`：生成当前 response 的原始 request 对象
- `flags (list)`：调试、标记、诊断用标志位列表
- `certificate`：服务端 SSL 证书对象
- `ip_address`：服务端 IP 地址
- `protocol (str)`：下载该 response 时使用的协议，如 `HTTP/1.1`、`h2`

额外约定：

- `response.meta` 必须等于该 response 对应 request 所携带的 `meta`
- `response.follow(..., meta=...)` 基于当前 `meta` 做合并或覆盖
- `body` 与 `text` 都保留；前者用于原始字节处理，后者用于文本解析

### 3.2 Response 解析 API

对外统一支持：

- `response.css(selector, options=None)`
- `response.xpath(selector, options=None)`
- `response.json(selector=None, options=None)`
- `response.xml(selector, options=None)`
- `response.regex(pattern, source="text", options=None)`
- `response.ai(prompt, source="html", options=None)`
- `response.ocr(selector=None, options=None)`

### 3.3 Request 核心属性

建议对外 `Request` 至少提供这些属性：

- `url`
- `mode`
- `method`
- `headers`
- `body`
- `meta`
- `callback`
- `dont_filter`
- `runtime`

说明：

- `url`：请求地址
- `mode`：请求模式；P0 必须支持 `http` 与 `browser`
- `method`：请求方法
- `headers`：请求头
- `body`：请求体
- `meta`：透传上下文
- `callback`：目标 callback；未显式指定时默认回到 `parse`
- `dont_filter`：是否跳过默认去重
- `runtime`：当前 request 的局部执行策略覆盖；第一版建议谨慎开放

建议把 request 统一抽象为两种模式：

- `mode = "http"`
- `mode = "browser"`

这两类 request 共用同一 `Request` 外部结构，但底层交给不同下载器执行。

### 3.4 默认 trim 规则

纯文本结果默认做前后空格清理：

- `response.css(...).text()`
- `response.xpath(...).text()`
- `response.regex(...).group(1)`
- `response.css(...).attr("href")`

以下结果默认不做 trim：

- `html()`
- `json().value()`
- 结构化对象值

如需关闭 trim，可通过 `options` 显式指定：

```python
response.regex(r"...", options={"trim": False}).group(1)
```

### 3.5 Scrapy 风格兼容

建议兼容：

- `response.css(".title::text").one()`
- `response.css(".list a::attr(href)").all()`

同时支持统一 API 风格：

- `response.css(".title").text()`
- `response.css(".list a").attr("href").all()`

### 3.6 follow 行为

推荐行为：

```python
yield response.follow(next_url)
yield response.follow(next_url, callback=self.parse_detail)
yield response.follow(next_url, callback=self.parse_detail, meta={"from_list": True})
```

规则：

- 默认 callback 为 `parse`
- 代码模式下 callback 推荐使用函数引用，不用字符串
- 如果未来支持 `next_step="detail"`，则它表示跳到 DSL step，不与 callback 混用

### 3.7 parse / callback 返回模型

对外推荐保持简单：

- `yield item`
- `yield request`

例如：

```python
async def parse(self, response):
    yield {"title": response.css("h1.title").text()}
    yield response.follow("/detail/1", callback=self.parse_detail)
```

不建议第一版向业务代码暴露过多控制流对象，如 `DropSignal`、`RetrySignal` 等。

---

## 4) Rules 统一设计

### 4.1 Spider 上只保留一个 `rules`

`rules` 统一设计为：

```json
{
  "type": "local",
  "options": {
    "path": "rules/myspider.json"
  }
}
```

### 4.2 统一原则

- `type` 决定规则来源
- `options` 承载该来源所需参数
- 最终加载结果必须是一份标准 JSON DSL

### 4.3 推荐的 `rules.type`

建议支持：

- `inline`
- `local`
- `redis`
- `db`
- `http`
- `custom`

默认推荐：

- 正式项目：`local`
- 简单 demo：`inline`
- 平台化/线上：`redis` 或 `db`

### 4.4 示例

#### 内联规则

```json
{
  "type": "inline",
  "options": {
    "value": {
      "steps": [
        {
          "id": "parse",
          "impl": "dsl",
          "fetch": {},
          "parse": {},
          "route": {},
          "output": {},
          "runtime": {}
        }
      ]
    }
  }
}
```

#### 本地文件

```json
{
  "type": "local",
  "options": {
    "path": "rules/myspider.json"
  }
}
```

#### Redis

```json
{
  "type": "redis",
  "options": {
    "dsn": "redis://127.0.0.1:6379/0",
    "key": "spider:rules:myspider"
  }
}
```

### 4.5 规则加载流程

统一流程：

1. 根据 `rules.type + rules.options` 读取规则源
2. 解析成 JSON DSL
3. 做 schema 校验
4. 补默认值并标准化
5. 返回统一规则 DSL

---

## 5) DSL 结构与代码共存规则

### 5.1 总体思路

对外代码用户主要面对的是 `Spider + parse + callback`，但内部统一执行单元仍然是 `step`。

`step` 主结构固定为：

- `fetch`
- `parse`
- `route`
- `output`
- `runtime`

### 5.2 默认入口规则

顶层 DSL 不再设计 `entry`。

统一规则改为：

- Spider 的默认入口永远是 `parse`
- 如果启用了 `rules` 并希望 DSL 直接接管默认入口，则 `steps` 中必须提供 `id = "parse"` 的 step
- 其他 step 通过 `next_step` 串联
- 不再通过 DSL 顶层 `entry` 再额外指定入口

这意味着：

- 纯代码 Spider：默认从代码 `parse` 开始
- 纯 DSL Spider：默认从 `steps["parse"]` 开始
- 混合模式：`parse` 可以是代码 callback，也可以是 DSL step；但默认入口名字始终固定为 `parse`

### 5.3 `fetch.mode`

`fetch.mode` 设计上必须支持：

- `http`
- `browser`

#### HTTP 模式

```json
{
  "fetch": {
    "mode": "http",
    "request": {
      "method": "GET",
      "timeout_ms": 15000,
      "headers": {},
      "body": null
    }
  }
}
```

#### Browser 模式

```json
{
  "fetch": {
    "mode": "browser",
    "browser": {
      "driver": "playwright",
      "engine": "chromium",
      "headless": true,
      "stealth": true,
      "fingerprint_profile": "desktop_zh_cn",
      "launch_options": {},
      "context_options": {},
      "page_options": {},
      "artifacts": {
        "screenshot": false,
        "html_snapshot": false
      }
    }
  }
}
```

浏览器模式当前设计目标包含：

- Playwright 驱动
- 支持 `Chromium` 与 `Google Chrome`
- 高级 stealth 能力
- fingerprint profile 能力
- 挑战页、验证页处理扩展点

说明：

- 这些能力在设计上需要预留，但不建议在设计稿里承诺“自动绕过所有类型的防护”
- `browser` 进入 P0，但实现上应先聚焦最小闭环，避免一开始做得过重

### 5.4 step 的 `impl`

每个 step 只能二选一：

- `impl = "dsl"`
- `impl = "code"`

#### DSL step

```json
{
  "id": "parse",
  "impl": "dsl",
  "fetch": {},
  "parse": {},
  "route": {},
  "output": {},
  "runtime": {}
}
```

#### 代码 step

```json
{
  "id": "detail",
  "impl": "code",
  "callback": "parse_detail",
  "fetch": {},
  "route": {},
  "output": {},
  "runtime": {}
}
```

约束：

- `impl=dsl` 时，不再写 `callback`
- `impl=code` 时，必须显式写 `callback`
- 不根据 `step.id` 自动猜 callback 方法名

### 5.5 parse 规则模型

`parse` 里固定为：

- `fields`
- `links`

#### 字段规则

```json
{
  "name": "title",
  "source": "html",
  "selector_type": "css",
  "selector": ["h1.title", ".article-title"],
  "attribute": "text",
  "required": true,
  "default": null,
  "multiple": false,
  "options": {}
}
```

#### 链接规则

```json
{
  "name": "detail_links",
  "source": "html",
  "selector_type": "css",
  "selector": [".article-list a.title"],
  "attribute": "attr:href",
  "required": false,
  "default": [],
  "multiple": true,
  "allow": ["^https://example\\.com/detail/\\d+$"],
  "deny": [],
  "to": {
    "next_step": "detail",
    "meta_patch": {
      "from_list": true
    }
  },
  "options": {}
}
```

### 5.6 `source` / `selector_type`

建议支持：

- `source`: `html / text / json / xml / headers / final_url / meta.xxx`
- `selector_type`: `css / xpath / json / xml / regex / ai / ocr`

### 5.7 `options` 作为统一扩展槽

`options` 用于承载：

- `css/xpath` 的附加参数
- `regex` 的 `flags`
- `ai` 的 `model / prompt / temperature`
- `ocr` 的 `lang / region`

不引入独立 `transforms` 配置块。复杂处理仍然优先回到代码 callback。

`ai` 解析的底层模型调用实现，后续可优先基于 Rust 库 `async-openai` 接入，作为统一 provider/client 的首选候选；当前阶段先固定对外 API 和执行链，不在这一轮主流程里提前实现具体模型调用细节。

---

## 6) Runtime 设计

### 6.1 `runtime` 的结构

推荐结构：

```json
{
  "runtime": {
    "schedule": {},
    "retry": {},
    "dedup": {}
  }
}
```

### 6.2 `schedule`

建议包含：

- `concurrency`
- `interval_ms`
- `rate_per_minute`

其中：

- `concurrency` 更偏执行器配额
- `interval_ms` / `rate_per_minute` 更偏请求准入策略

### 6.3 `retry`

建议包含：

- `count`
- `http_status`
- `backoff_ms`

### 6.4 `dedup`

建议包含：

- `enabled`
- `key`
- `ttl_sec`
- `scope`

约束：

- `dedup.key` 只能引用下载前可获得的数据，如 `url`、`meta.xxx`

### 6.5 与 middleware 的关系

`runtime` 负责高层语义，最终会被编译成默认 middleware 配置，例如：

- `retry` -> `retry_by_status` / `retry_by_error`
- `dedup` -> `dedup`
- `schedule.interval_ms` -> `interval_gate`
- `schedule.rate_per_minute` -> `rate_limit`

---

## 7) MIDDLEWARES 最终配置

### 7.1 统一结构

`MIDDLEWARES` 统一为一个配置块：

```json
{
  "MIDDLEWARES": {
    "retry_by_status": {
      "enabled": true,
      "type": "download",
      "order": 200,
      "options": {
        "status": [429, 500, 502, 503, 504]
      }
    }
  }
}
```

### 7.2 每项固定字段

- `enabled`
- `type`
- `order`
- `options`

### 7.3 `type`

第一版建议只支持：

- `download`
- `spider`

#### `download`

用于：

- 请求前处理
- 响应后处理
- 异常处理
- retry / dedup / cookies / proxy / rate_limit

#### `spider`

用于：

- response 进入 parse 前后
- parse 结果进入后续处理前后

### 7.4 key 建议

默认推荐使用短逻辑名：

- `retry_by_status`
- `cookies`
- `proxy`
- `referer`
- `custom_signature`

不推荐把完整类路径当作主配置 key。

### 7.5 覆盖规则

`MIDDLEWARES` 的 key 是唯一标识。

- 同名 key 视为覆盖同一个 middleware 配置
- `enabled = false` 表示禁用
- 显式 `MIDDLEWARES` 配置可覆盖 `runtime` 自动生成的默认 middleware 配置

---

## 8) Middleware 方法命名与边界

### 8.1 middleware 方法

middleware 接口方法统一使用动词式命名：

- `process_request`
- `process_response`
- `process_exception`

推荐 contract：

```text
process_request(ctx) -> Continue | ReplaceRequest(request) | Respond(response_stub) | Drop(reason)
process_response(ctx, response) -> Continue(response) | ReplaceResponse(response) | Retry(backoff_ms, reason) | Drop(reason)
process_exception(ctx, error) -> Retry(backoff_ms, reason) | Respond(response_stub) | Drop(reason) | Unhandled
```

### 8.2 callback / middleware / event 的边界

- `callback`：决定下一步由哪个代码处理方法接手
- `middleware`：请求生命周期拦截与改写
- `event`：生命周期通知与观察点

一句话：

- callback 决定“谁处理”
- middleware 决定“怎么拦截处理”
- event 决定“发生了什么并通知谁”

### 8.3 事件点命名

事件点不加 `on`，例如：

- `request_started`
- `response_received`
- `item_emitted`

如果未来设计监听器方法，方法名可以再用 `on_xxx`。

---

## 9) 插件机制

### 9.1 采用方式

采用：

- 插件清单 + 自动加载

推荐使用：

- `plugins.toml`

### 9.2 为什么需要插件机制

最终配置用户只关心：

- 配置里有哪些 middleware / rules / provider / storage 可以用

而不关心：

- 它们背后是哪段代码
- 如何手动注册
- 如何实例化

因此需要由框架在启动阶段自动装载可用能力。

### 9.3 插件类别

插件类别第一版建议统一为：

- `middleware`
- `rules`
- `provider`
- `storage`

说明：

- `rules` 用于规则来源加载能力，命名与代码中的 `rules` 保持一致
- `callback` 第一版不进入插件系统，仍然作为 Spider 方法存在

### 9.4 插件清单示例

```toml
[[plugins]]
name = "custom_signature"
kind = "middleware"
entry = "myproject.plugins.custom_signature:Plugin"
override = false

[[plugins]]
name = "local"
kind = "rules"
entry = "myproject.plugins.local_rules:Plugin"
override = false

[[plugins]]
name = "redis"
kind = "rules"
entry = "myproject.plugins.redis_rules:Plugin"
override = false
```

### 9.5 `(kind, name)` 唯一识别

插件系统按 `(kind, name)` 注册，而不是全局单名字空间。

允许：

- `middleware.redis`
- `rules.redis`

二者不冲突。

### 9.6 同名覆盖规则

这是硬规则：

> 当插件名称与同类内置实现冲突时，只有显式声明 `override = true` 才允许覆盖；否则在加载阶段直接报错。

例如：

- `middleware.retry_by_status` 与内置同名
- 若未显式 `override = true`，则直接报错

### 9.7 自定义 middleware 如何生效

正确流程：

1. 项目开发者实现插件
2. 在 `plugins.toml` 里声明
3. 框架启动时自动加载
4. 配置用户在 `MIDDLEWARES` 中显式启用

例如：

```json
{
  "MIDDLEWARES": {
    "custom_signature": {
      "enabled": true,
      "type": "download",
      "order": 300,
      "options": {
        "app_id": "xxx"
      }
    }
  }
}
```

配置用户不负责主动注册。

### 9.8 未找到插件时的行为

如果配置引用了某个：

- `rules.type`
- `MIDDLEWARES key`
- `provider`
- `storage`

但插件系统里没有对应实现，则在加载/校验阶段直接报错，不允许静默跳过。

---

## 10) 覆盖优先级

推荐的覆盖顺序，从低到高：

1. 框架内建默认值
2. Spider 默认配置
3. `rules` 中的全局配置
4. `rules.steps[*]` 的 step 级配置
5. `response.follow(..., callback=...)` 或显式 `next_step`

核心原则：

- 显式优先于默认
- 越接近具体 step，优先级越高

---

## 11) 内部统一执行模型

虽然对外体验偏 Scrapy，但对内仍然建议统一为一套执行模型。

### 11.1 Scheduler

Scheduler 只做调度：

- ready / delayed / inflight
- lease / ack / nack / backoff
- priority / fairness

不负责：

- 解析
- 去重
- 输出

### 11.2 Engine

Engine 负责：

1. 从 Scheduler 取 URL 任务
2. 组装请求上下文
3. 执行 middleware 链
4. 下载页面
5. 调用代码 callback 或 DSL step
6. 产出 item 或下一跳 request
7. 做 ack / nack / retry

补充约束：

- `execute_spider_once()` 这类名字如果作为阶段性实现出现，可以接受，但后续主循环成型后，应优先收敛成更清晰的 `run()` 语义，或者直接并入最终 engine 主循环，不建议作为最终长期 API/架构命名保留。

### 11.3 统一解析能力

无论 DSL 还是代码模式，底层都复用同一套解析能力：

- `css`
- `xpath`
- `json`
- `xml`
- `regex`
- `ai`
- `ocr`

### 11.4 统一结果协议

即使对外代码用户主要写 `yield item` 和 `yield request`，内部仍建议统一成一套可执行结果协议，供：

- 代码 callback
- DSL step
- middleware
- 调度器

共同消费。

---

## 12) 默认推荐

当前最终推荐口径如下：

- 对外主入口：`Spider`
- 默认代码入口：`parse`
- 其他 callback：用户自定义
- callback 在代码模式下优先使用函数引用
- 规则入口统一为：`rules = { type, options }`
- 默认规则来源：`type = local`
- 文本解析默认 trim，必要时可 `options={"trim": False}`
- 中间件统一配置块：`MIDDLEWARES`
- `MIDDLEWARES` 每项固定为：`enabled / type / order / options`
- 中间件类别先支持：`download / spider`
- 插件机制采用：`plugins.toml + 自动加载`
- 插件类别统一为：`middleware / rules / provider / storage`
- 同类同名覆盖必须显式 `override = true`

---

## 13) 当前不作为默认推荐的内容

这些能力可以保留，但不作为当前主推荐设计：

- 自动根据 step id 猜 callback
- 让配置用户手动注册 middleware
- 把 callback 放进插件系统
- 独立 `transforms` DSL
- 过多业务控制流对象暴露给 `parse`
- 把 `Crawler / Registry / TaskConfig` 强行做成用户主 API

---

## 14) 下一步实现前建议

在真正开始编码前，建议继续做两件事：

1. 依据本设计稿，把最终 JSON DSL schema 定稿
2. 再把 Rust 实现时的内部 trait / registry / plugin loader 映射关系整理一版

Rust 实现层面建议遵循 Rust 2024 风格模块布局：

- 不使用 `mod.rs`
- 采用 `foo.rs` 与 `foo/` 子模块并存的组织方式
- 模块边界优先清晰，再考虑进一步细分

当前文档的目标已经不是“继续发散”，而是作为实现前的统一口径。

---

## 15) 实现优先级建议

为了避免第一版范围失控，建议按优先级分三层推进。

### 15.1 P0：第一版必须实现

这些能力构成最小可用内核：

- `Spider`
- 默认代码入口 `parse`
- `response.css / xpath / json / xml / regex / ai / follow`
- `Request / Response` 基础字段：`url / status / headers / body / text / meta / request`
- `Request.mode = http | browser`
- `rules = { type, options }`
- `rules.type = local / inline`
- DSL 基础结构：`steps / parse.fields / parse.links / runtime`
- `runtime` 基础能力：`schedule / retry / dedup`
- `MIDDLEWARES` 基础结构：`enabled / type / order / options`
- `download / spider` 两类 middleware
- 内建 middleware：`retry_by_status / retry_by_error / dedup / rate_limit / cookies / proxy`
- `plugins.toml + 自动加载`
- 插件类别至少支持：`middleware / rules`
- 同类同名覆盖规则：必须显式 `override = true`
- 浏览器下载模式 `browser`，包含 Playwright、Chromium/Google Chrome、stealth、fingerprint 与挑战页处理扩展

### 15.2 P1：第二版建议补齐

这些能力会显著增强通用性，但不是第一版必须：

- `response.ocr`
- `rules.type = redis / db / http`
- `response.ai` 的真实 provider 接入，优先评估基于 `async-openai` 实现底层模型调用
- `provider / storage` 插件类别
- step 级更细粒度覆盖与合并规则
- 更完整的事件系统
- 输出 pipeline 的细化配置

### 15.3 P2：后续再考虑

这些能力先保留设计，不建议第一版实现：

- callback 进入插件系统
- 复杂业务控制流对象直接暴露给 `parse`
- 更重的动态热更新与远程插件管理
- 过度灵活的 DSL 扩展语法
- 为所有能力设计完全自由的 override/merge 机制

### 15.4 推荐落地顺序

建议实现顺序如下：

1. `Spider / Request / Response` 对外 API
2. `parse` 与 callback 执行链
3. `rules.local / rules.inline` 加载与 DSL 校验
4. `runtime` 编译为默认 middleware
5. `MIDDLEWARES` 显式配置与覆盖
6. `plugins.toml` 自动加载
7. Scheduler / Engine / ack / nack / retry 主链路
8. 再补充 `redis/db/http` 规则源与 AI/OCR/provider 扩展
