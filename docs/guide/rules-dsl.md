# 05 Rules DSL 设计（v1）

本章给出 rules DSL v1 设计稿。

本版 DSL 采用更直观的“单链路流转模型”：

- 从 `seed` 开始发起一条链路
- 链路进入某个 `step`
- `step` 可以继续 `follow` 生成下一跳子链路
- 也可以在 `output` 处产出最终结果

本版重点是把以下几类能力拆清楚：

- 请求层：`request / engine(dedup / concurrency / interval / rate_limit / auto_throttle / retry_by_status / retry_by_error) / allow_url_pattern`
- 页面解析层：`fields / bind`
- 链路上下文：`meta`
- 结果输出层：`output / validate / sinks`

当前实现边界：

- `seed.engine.*` / `follow.engine.*` 已经编译为 request 级 middleware override，并沿现有 enqueue / download / retry 主链执行。
- `allow_url_pattern` 已在 `src/rules/run.rs` 中真实执行。
- `output.validate.required`、`output.validate.fields` 已经编译为 step 级共享 validator，并在 engine 的 `item -> pipeline -> validator -> store` 主链里真实执行。
- `seeds` 已经接入引擎起始请求生成；当 `rules.seeds` 非空时，engine 会以它为真正的启动请求来源；当 `rules.seeds` 为空时，仍回退到 `Spider::build_start_requests()` / `start_urls()`。
- `output.sinks` 已经接入运行时路由：rules 编译结果会保留 sink 名称，engine 在构建 step 运行时阶段把它解析为目标 store 实例列表，并 fan-out 到已注册的 store。
- 顶层 `sinks` 目前仍只负责 DSL 注册表、结构校验和引用校验；它不会自动实例化底层 store，真实 store 仍由业务侧通过 `Engine::with_store(...)` / `Engine::with_stores(...)` 注入。

---

## 05.1 设计原则

### 05.1.1 单链路模型

DSL 描述的是“一条数据如何从起点走到最终结果”。

例如：

```text
start_url -> 列表页 -> 某一个 detail_url -> 这一条 detail_url 的完整结果
```

不是：

```text
start_url -> 列表页 -> 一批 detail_url -> 一批结果
```

列表页如果发现多条详情链接，语义是“当前链路分裂成多条子链路”，而不是让用户在 DSL 里写批处理循环。

### 05.1.2 `meta` 属于当前链路

`meta` 表示绑定到当前下一跳请求上的上下文。

常见场景：

- 列表页已经拿到了 `title`
- 列表页已经拿到了 `cover`
- 列表页已经拿到了 `rank / channel / edition_id`

这些值都应该随着当前这条详情页请求一起往下传，而不是到下一步再重复解析。

### 05.1.3 请求层与结果层分离

请求相关能力放在请求链路上：

- `request`
- `engine.dedup`
- `engine.concurrency`
- `engine.interval`
- `engine.rate_limit`
- `engine.auto_throttle`
- `engine.retry_by_status`
- `engine.retry_by_error`
- `allow_url_pattern`

结果相关能力放在输出层：

- `output.item`
- `output.validate`
- `output.sinks`

### 05.1.4 配置集中定义、局部单值引用

本版 engine middleware 统一采用“顶层注册表 + 局部单值引用”的方式：

- `engine.dedup`
- `engine.concurrency`
- `engine.interval`
- `engine.rate_limit`
- `engine.auto_throttle`
- `engine.retry_by_status`
- `engine.retry_by_error`

这样配置不会分散，也方便统一维护。

### 05.1.5 统一取值模型

凡是“字段怎么取值”，尽量都使用统一的值模型：

- 字面量
- `from`
- `template + vars`
- `transforms`
- `fallback`

这样 `bind / request / meta / output.item` 的心智模型是一套。

---

## 05.2 顶层结构

```yaml
spider:
  name: "..."
  clock: ...

engine:
  dedup: ...
  concurrency: ...
  interval: ...
  rate_limit: ...
  auto_throttle: ...
  retry_by_status: ...
  retry_by_error: ...

sinks:
  ...

seeds:
  - ...

steps:
  - ...
```

| 顶层字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `spider` | object | 是 | 无 | 爬虫级基础配置 |
| `engine` | object | 否 | 无 | engine 共享能力配置入口 |
| `sinks` | object | 否 | 无 | 输出目标注册表 |
| `seeds` | array<object> | 是 | 无 | 起始请求列表 |
| `steps` | array<object> | 是 | 无 | 页面处理步骤列表 |

顶层块含义：

- `spider`
  爬虫级基础配置。
- `engine`
  engine 共享能力入口，直接挂当前真实 middleware 注册表。
- `sinks`
  输出目标注册表。
- `seeds`
  起始请求。
- `steps`
  页面处理步骤。

---

## 05.3 完整示例

```yaml
spider:
  name: "shxwcb"

  clock:
    timezone: "Asia/Shanghai"

engine:
  concurrency:
    spider_global:
      bucket: "spider"
      concurrency: 5

    detail_serial:
      bucket: "origin"
      concurrency: 1

  interval:
    origin_guard:
      bucket: "origin"
      interval: 800

  rate_limit:
    origin_budget:
      bucket: "origin"
      rate_per_minute: 120

  retry_by_status:
    default_http_retry:
      count: 3
      status: [429, 500, 502, 503, 504]
      backoff: [1000, 3000, 5000]

  retry_by_error:
    default_error_retry:
      count: 2
      backoff: [1000, 3000]

    detail_error_retry:
      count: 4
      backoff: [1000, 3000, 5000, 8000]

  dedup:
    request_url:
      backend: "memory"
      key: ["url"]

    request_url_with_edition:
      backend: "memory"
      key: ["url", "meta.edition_id"]

sinks:
  article_db:
    type: "db"
    table: "articles"
    mode: "upsert"
    unique_keys: ["source_url"]

  article_file:
    type: "file"
    format: "jsonl"
    path: "./output/articles.jsonl"

seeds:
  - id: "period_index"

    request:
      method: "GET"
      url:
        template: "https://ep.shxwcb.com/{year}/{month}/period.xml"
        vars:
          year:
            from: "$now"
            transforms:
              - type: "date_format"
                format: "%Y"
          month:
            from: "$now"
            transforms:
              - type: "date_format"
                format: "%m"
      timeout: 10000

    next_step: "parse_period_xml"

steps:
  - id: "parse_period_xml"

    fields:
      period_date:
        selector: "//period[last()]"
        attr: "date"

      edition_id:
        selector: "//period[last()]"
        attr: "id"

      page_no:
        selector: "//period[last()]"
        attr: "number"

    bind:
      year:
        from: "$fields.period_date"
        transforms:
          - type: "date_format"
            input_format: "%Y-%m-%d"
            format: "%Y"

      month:
        from: "$fields.period_date"
        transforms:
          - type: "date_format"
            input_format: "%Y-%m-%d"
            format: "%m"

      day:
        from: "$fields.period_date"
        transforms:
          - type: "date_format"
            input_format: "%Y-%m-%d"
            format: "%d"

    follow:
      - next_step: "parse_front_page"

        request:
          url:
            template: "https://ep.shxwcb.com/{year}/{month}/{day}/{edition_id}__{page_no}.html"
            vars:
              year:
                from: "$bind.year"
              month:
                from: "$bind.month"
              day:
                from: "$bind.day"
              edition_id:
                from: "$bind.edition_id"
              page_no:
                from: "$bind.page_no"

        meta:
          period_date:
            from: "$fields.period_date"
          edition_id:
            from: "$fields.edition_id"
          page_no:
            from: "$fields.page_no"

        engine:
          dedup: "request_url"

  - id: "parse_front_page"

    follow:
      - item: ".news-list li"
        next_step: "parse_detail"

        request:
          url:
            selector: "a"
            attr: "href"

        meta:
          title:
            selector: "a"
            attr: "title"

          cover:
            selector: "img"
            attr: "src"

          edition_id:
            from: "$meta.edition_id"

        allow_url_pattern:
          - "^https?://"

        engine:
          concurrency: "detail_serial"
          interval: "origin_guard"
          rate_limit: "origin_budget"
          retry_by_status: "default_http_retry"
          retry_by_error: "detail_error_retry"
          dedup: "request_url_with_edition"

  - id: "parse_detail"

    fields:
      title:
        selector: "h1"
        text: true

      publish_time:
        selector: ".date"
        text: true

      content:
        selector: ".article-content"
        html: true

    output:
      item:
        title:
          from: "$meta.title"
          fallback:
            from: "$fields.title"

        cover:
          from: "$meta.cover"

        publish_time:
          from: "$fields.publish_time"

        content:
          from: "$fields.content"

        source_url:
          from: "$response.url"

      validate:
        required:
          - "title"
          - "content"
          - "source_url"

        fields:
          title:
            type: "string"
            min_length: 1
            max_length: 120

          content:
            type: "string"
            min_length: 20

          source_url:
            type: "string"
            format: "url"

      sinks:
        - "article_db"
        - "article_file"
```

---

## 05.4 `spider`

```yaml
spider:
  name: "shxwcb"

  clock:
    timezone: "Asia/Shanghai"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `spider.name` | string | 是 | 无 | 爬虫名称 |
| `spider.clock` | object | 否 | 无 | 时间相关配置 |
| `spider.clock.timezone` | string | 否 | 无 | `$now` 等时间变量的计算时区，建议显式指定 |

字段说明：

- `name`
  爬虫名称。
- `clock.timezone`
  时间变量的计算时区，例如 `$now`。

说明：

- `spider` 只放爬虫级基础信息。
- 不把 `engine / sinks / steps` 混到这里。

---

## 05.5 `engine`

### 05.5.1 顶层结构

```yaml
engine:
  dedup: ...
  concurrency: ...
  interval: ...
  rate_limit: ...
  auto_throttle: ...
  retry_by_status: ...
  retry_by_error: ...
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `engine` | object | 否 | 无 | engine 共享能力入口 |
| `engine.dedup` | object | 否 | 无 | 请求入队前去重配置注册表 |
| `engine.concurrency` | object | 否 | 无 | 下载前并发控制配置注册表 |
| `engine.interval` | object | 否 | 无 | 下载前固定间隔控制配置注册表 |
| `engine.rate_limit` | object | 否 | 无 | 下载前速率限制配置注册表 |
| `engine.auto_throttle` | object | 否 | 无 | 下载前自适应节流配置注册表 |
| `engine.retry_by_status` | object | 否 | 无 | 按响应状态码重试配置注册表 |
| `engine.retry_by_error` | object | 否 | 无 | 按下载异常重试配置注册表 |

### 05.5.2 为什么直接用 middleware 名称

本版文档改为按当前代码里真实存在的内置 middleware key 反向驱动，不再额外包装成 `schedule` 或组级运行策略抽象层。

也就是说：

- DSL 中 `engine` 下的字段名，直接对应底层 middleware 名称
- 顶层 `engine.<middleware_name>` 负责注册命名配置
- `seed.engine.<middleware_name>` / `follow.engine.<middleware_name>` 只负责引用名字

这样做的好处是：

- DSL 与底层实现同名，减少歧义
- rules 编译时不需要再做一层概念翻译
- 用户看到 DSL 字段名，就能直接对应到实际中间件能力

### 05.5.3 关于 `schedule`

`schedule` 在当前代码里不是一个独立的 middleware key，也不是一个单独的配置块。

与下载前调度控制相关的能力，直接使用这些真实 middleware：

- `concurrency`
- `interval`
- `rate_limit`
- `auto_throttle`

下载前调度控制直接围绕这些 concrete middleware 配置展开。

### 05.5.4 `bucket` 规则

DSL authored 层统一使用 `bucket` 表示下载前中间件的分桶维度，适用于：

- `engine.concurrency`
- `engine.interval`
- `engine.rate_limit`
- `engine.auto_throttle`

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `engine.<download_middleware>.<name>.bucket` | string / array<string> | 否 | `origin` | 当前 middleware 的分桶维度 |

说明：

- DSL 中写 `bucket`

当前代码支持的 `bucket` token：

- `spider`
- `origin`
- `domain`
- `url`
- `method`
- `meta.xxx`

说明：

- `bucket: "origin"` 表示同源站共用一个桶
- `bucket: "spider"` 表示整个 spider 共用一个桶
- `bucket: ["origin", "meta.channel"]` 表示按复合维度分桶

### 05.5.5 `engine.concurrency`

```yaml
engine:
  concurrency:
    spider_global:
      bucket: "spider"
      concurrency: 5

    detail_serial:
      bucket: "origin"
      concurrency: 1
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `engine.concurrency.<name>.bucket` | string / array<string> | 否 | `origin` | 并发控制分桶维度 |
| `engine.concurrency.<name>.concurrency` | int | 是 | 无 | 每个桶允许的最大并发数 |

### 05.5.6 `engine.interval`

```yaml
engine:
  interval:
    origin_guard:
      bucket: "origin"
      interval: 800
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `engine.interval.<name>.bucket` | string / array<string> | 否 | `origin` | 间隔控制分桶维度 |
| `engine.interval.<name>.interval` | number | 是 | 无 | 同一桶两次请求之间的最小间隔，单位 `ms` |

### 05.5.7 `engine.rate_limit`

```yaml
engine:
  rate_limit:
    origin_budget:
      bucket: "origin"
      rate_per_minute: 120
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `engine.rate_limit.<name>.bucket` | string / array<string> | 否 | `origin` | 速率限制分桶维度 |
| `engine.rate_limit.<name>.rate_per_minute` | int | 是 | 无 | 每个桶每分钟允许的请求数 |

### 05.5.8 `engine.auto_throttle`

```yaml
engine:
  auto_throttle:
    origin_adaptive:
      bucket: "origin"
      target_concurrency: 1.5
      start_interval: 300
      min_interval: 100
      max_interval: 3000
      error_backoff_ratio: 2.0
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `engine.auto_throttle.<name>.bucket` | string / array<string> | 否 | `origin` | 自适应节流分桶维度 |
| `engine.auto_throttle.<name>.target_concurrency` | number | 否 | `1.0` | 目标并发度 |
| `engine.auto_throttle.<name>.start_interval` | number | 否 | `0` | 初始间隔，单位 `ms` |
| `engine.auto_throttle.<name>.min_interval` | number | 否 | `0` | 最小间隔，单位 `ms` |
| `engine.auto_throttle.<name>.max_interval` | number | 否 | `60000` | 最大间隔，单位 `ms` |
| `engine.auto_throttle.<name>.error_backoff_ratio` | number | 否 | `2.0` | 下载异常时的退避倍率 |

### 05.5.9 局部引用

`seed` 和 `follow` 可以按真实 middleware 名称引用下载前能力：

```yaml
follow:
  - next_step: "parse_detail"
    engine:
      concurrency: "detail_serial"
      interval: "origin_guard"
      rate_limit: "origin_budget"
      auto_throttle: "origin_adaptive"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `seed.engine.concurrency` | string | 否 | 无 | 引用一条 `engine.concurrency` 配置 |
| `seed.engine.interval` | string | 否 | 无 | 引用一条 `engine.interval` 配置 |
| `seed.engine.rate_limit` | string | 否 | 无 | 引用一条 `engine.rate_limit` 配置 |
| `seed.engine.auto_throttle` | string | 否 | 无 | 引用一条 `engine.auto_throttle` 配置 |
| `follow.engine.concurrency` | string | 否 | 无 | 引用一条 `engine.concurrency` 配置 |
| `follow.engine.interval` | string | 否 | 无 | 引用一条 `engine.interval` 配置 |
| `follow.engine.rate_limit` | string | 否 | 无 | 引用一条 `engine.rate_limit` 配置 |
| `follow.engine.auto_throttle` | string | 否 | 无 | 引用一条 `engine.auto_throttle` 配置 |

约束：

- 每个字段都是单值字符串，不支持数组
- 字段名和顶层注册表同名，不做跨类型引用
- `seed / follow` 里只引用名字，不重复内联具体参数

---

## 05.6 `engine.retry_by_status / engine.retry_by_error`

### 05.6.1 顶层结构

```yaml
engine:
  retry_by_status:
    default_http_retry:
      count: 3
      status: [429, 500, 502, 503, 504]
      backoff: [1000, 3000, 5000]

  retry_by_error:
    default_error_retry:
      count: 2
      backoff: [1000, 3000]
```

### 05.6.2 `engine.retry_by_status`

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `engine.retry_by_status.<name>.count` | int | 是 | 无 | 最大重试次数 |
| `engine.retry_by_status.<name>.status` | array<int> | 否 | `[]` | 触发重试的 HTTP 状态码 |
| `engine.retry_by_status.<name>.backoff` | array<number> | 否 | `[]` | 各次重试的等待时间，单位 `ms` |

说明：

- 文档推荐使用 `status`

### 05.6.3 `engine.retry_by_error`

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `engine.retry_by_error.<name>.count` | int | 是 | 无 | 最大重试次数 |
| `engine.retry_by_error.<name>.backoff` | array<number> | 否 | `[]` | 各次重试的等待时间，单位 `ms` |

### 05.6.4 局部引用

```yaml
follow:
  - next_step: "parse_detail"
    engine:
      retry_by_status: "default_http_retry"
      retry_by_error: "detail_error_retry"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `seed.engine.retry_by_status` | string | 否 | 无 | 引用一条 `engine.retry_by_status` 配置 |
| `seed.engine.retry_by_error` | string | 否 | 无 | 引用一条 `engine.retry_by_error` 配置 |
| `follow.engine.retry_by_status` | string | 否 | 无 | 引用一条 `engine.retry_by_status` 配置 |
| `follow.engine.retry_by_error` | string | 否 | 无 | 引用一条 `engine.retry_by_error` 配置 |

### 05.6.5 `backoff` 规则

- `count` 表示最多重试几次
- `backoff` 可以比 `count` 短
- 如果 `count` 大于 `backoff` 长度，超出的重试继续使用最后一个 `backoff` 值

例如：

```yaml
count: 4
backoff: [1000, 3000]
```

则表示：

- 第 1 次重试等待 1000ms
- 第 2 次重试等待 3000ms
- 第 3 次重试等待 3000ms
- 第 4 次重试等待 3000ms

---

## 05.7 `engine.dedup`

### 05.7.1 顶层结构

```yaml
engine:
  dedup:
    request_url:
      backend: "memory"
      key: ["url"]

    request_url_with_edition:
      backend: "memory"
      key: ["url", "meta.edition_id"]
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `engine.dedup.<name>.backend` | string | 否 | `memory` | 去重后端，当前内置支持 `memory / bloom / noop` |
| `engine.dedup.<name>.key` | array<string> | 否 | `["url"]` | 去重指纹字段列表 |
| `engine.dedup.<name>.expected_items` | int | 否 | `100000` | 当 `backend=bloom` 时的预计元素数 |
| `engine.dedup.<name>.false_positive_rate` | number | 否 | `0.01` | 当 `backend=bloom` 时的误判率 |

说明：

- 当前代码同时接受 `key` 和 `keys`，文档统一推荐写 `key`
- 如果不写 `key`，默认等价于 `["url"]`

### 05.7.2 局部引用

```yaml
follow:
  - next_step: "parse_detail"
    engine:
      dedup: "request_url_with_edition"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `seed.engine.dedup` | string | 否 | 无 | 引用一条 `engine.dedup` 配置 |
| `follow.engine.dedup` | string | 否 | 无 | 引用一条 `engine.dedup` 配置 |

### 05.7.3 `key` 怎么算

`key` 中的值按顺序取出，然后拼成最终去重指纹。

例如：

```yaml
key:
  - "url"
  - "meta.edition_id"
```

则最终可规范化为：

```text
url={request.url}|meta.edition_id={meta.edition_id}
```

当前代码支持的 key token：

- `url`
- `method`
- `body`
- `meta.xxx`

### 05.7.4 当前不支持 `ttl`

当前内置 dedup 没有 `ttl` 字段，也不会按时间自动过期。

这意味着：

- `dedup` 只负责“这条请求要不要进入队列”
- 不负责“过几天自动解除去重”
- 也不负责结果层幂等

如果后续需要“7 天去重”这类时效能力，应通过自定义 dedup backend 实现，而不是先在 DSL 文档里写一个运行时还不支持的 `ttl`。

---

## 05.8 `sinks`

```yaml
sinks:
  article_db:
    type: "db"
    table: "articles"
    mode: "upsert"
    unique_keys: ["source_url"]

  article_file:
    type: "file"
    format: "jsonl"
    path: "./output/articles.jsonl"

  article_mq:
    type: "mq"
    topic: "spider.article"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `sinks.<name>.type` | string | 是 | 无 | 输出类型，当前建议支持 `db / file / mq` |
| `sinks.<name>.table` | string | 条件必填（`type=db`） | 无 | 目标表名 |
| `sinks.<name>.mode` | string | 否 | 无 | 数据库写入模式，例如 `upsert` |
| `sinks.<name>.unique_keys` | array<string> | 否 | 无 | 数据库幂等字段，常用于 `upsert` |
| `sinks.<name>.format` | string | 条件必填（`type=file`） | 无 | 文件输出格式，例如 `jsonl` |
| `sinks.<name>.path` | string | 条件必填（`type=file`） | 无 | 文件输出路径 |
| `sinks.<name>.topic` | string | 条件必填（`type=mq`） | 无 | MQ topic |

字段说明：

- `sinks.<name>`
  一个命名输出目标。
- `type`
  输出类型，例如 `db / file / mq`。
- 其余字段为该 sink 类型自己的配置。

说明：

- `sinks` 是注册表。
- `output.sinks` 里只引用名字。
- 进入 runtime 后，`output.sinks` 会被解析为当前 step 的目标 store 列表。
- v1 不再单独写 `mapping`，因为 `output.item` 已经决定了最终输出结构。

---

## 05.9 `seeds`

```yaml
seeds:
  - id: "period_index"

    request:
      url: ...

    meta:
      source: "shxwcb"

    allow_url_pattern:
      - "^https?://"

    engine:
      concurrency: "spider_global"
      interval: "origin_guard"
      rate_limit: "origin_budget"
      retry_by_status: "default_http_retry"
      retry_by_error: "default_error_retry"
      dedup: "request_url"

    next_step: "parse_period_xml"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `seeds[].id` | string | 是 | 无 | seed 唯一标识 |
| `seeds[].request` | object | 是 | 无 | 起始请求配置 |
| `seeds[].meta` | object | 否 | 无 | 初始链路上下文 |
| `seeds[].allow_url_pattern` | array<string> | 否 | 无 | URL 过滤规则 |
| `seeds[].engine.concurrency` | string | 否 | 无 | 引用一条 `engine.concurrency` 配置 |
| `seeds[].engine.interval` | string | 否 | 无 | 引用一条 `engine.interval` 配置 |
| `seeds[].engine.rate_limit` | string | 否 | 无 | 引用一条 `engine.rate_limit` 配置 |
| `seeds[].engine.auto_throttle` | string | 否 | 无 | 引用一条 `engine.auto_throttle` 配置 |
| `seeds[].engine.retry_by_status` | string | 否 | 无 | 引用一条 `engine.retry_by_status` 配置 |
| `seeds[].engine.retry_by_error` | string | 否 | 无 | 引用一条 `engine.retry_by_error` 配置 |
| `seeds[].engine.dedup` | string | 否 | 无 | 当前 seed 额外挂一条去重规则 |
| `seeds[].next_step` | string | 是 | 无 | 请求成功后进入的 step |

字段说明：

- `id`
  seed 唯一标识。
- `request`
  起始请求配置。
- `meta`
  初始链路上下文，可选。
- `allow_url_pattern`
  URL 过滤规则，可选。
- `engine.concurrency / engine.interval / engine.rate_limit / engine.auto_throttle`
  额外挂下载前 middleware 配置，可选。
- `engine.retry_by_status / engine.retry_by_error`
  额外挂重试 middleware 配置，可选。
- `engine.dedup`
  额外挂一条去重规则，可选。
- `next_step`
  请求成功后进入哪个 step。

说明：

- seed 本质上也是一条请求。
- 所以它和 follow 一样，可以挂请求层能力。
- `engine.*` 这里只写规则名，具体规则统一在顶层 `engine` 注册表维护。

当前实现说明：

- 当 `rules.seeds` 非空时，引擎会直接用它们生成真正的起始请求。
- 当 `rules.seeds` 为空时，引擎回退到 `Spider::build_start_requests()` / `start_urls()`。
- `seed.request` 和 `follow.request` 走同一套 request plan 求值与构建语义。

---

## 05.10 `steps`

```yaml
steps:
  - id: "parse_detail"

    fields: ...
    bind: ...
    follow: ...
    output: ...
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `steps[].id` | string | 是 | 无 | step 标识 |
| `steps[].fields` | object | 否 | 无 | 当前页面直接解析出的字段 |
| `steps[].bind` | object | 否 | 无 | 当前页面中间变量 |
| `steps[].follow` | array<object> | 否（与 `output` 至少一个） | 无 | 当前页面继续生成下一跳 |
| `steps[].output` | object | 否（与 `follow` 至少一个） | 无 | 当前页面产出最终结果 |

字段说明：

- `id`
  step 标识。
- `fields`
  当前页面直接解析出的字段。
- `bind`
  当前页面基于已有值计算出来的中间变量。
- `follow`
  从当前链路继续生成下一跳子链路。
- `output`
  当前链路到这里时产出的最终结果。

说明：

- 一个 step 至少要有 `follow` 或 `output` 之一。
- `step` 自己不放 `engine.*` 的具体 middleware 参数。
- 请求控制只放在 `seed / follow` 的 `engine` 上。

---

## 05.11 `fields`

### 05.11.1 页面级解析

```yaml
fields:
  title:
    selector: "h1"
    text: true

  content:
    selector: ".article-content"
    html: true

  detail_url:
    selector: "a"
    attr: "href"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `fields.<name>.selector` | string | 是 | 无 | 当前页面中的提取表达式 |
| `fields.<name>.text` | bool | 否 | 实际默认提取文本 | 取文本 |
| `fields.<name>.html` | bool | 否 | `false` | 取 HTML |
| `fields.<name>.attr` | string | 否 | 无 | 取指定属性 |

字段说明：

- `selector`
  当前页面中的提取表达式。
- `text: true`
  取文本。
- `html: true`
  取 HTML。
- `attr: "<name>"`
  取指定属性。

约束：

- `text / html / attr` 一次只能写一种。
- 如果三者都不写，当前实现默认按 `text` 提取。
- v1 的 `fields` 只负责提取值，不暴露 `node` 对象。
- 节点级作用域统一由 `follow.item` 表达。
- `fields` 解析结果统一通过 `$fields.xxx` 引用。

### 05.11.2 关于选择器

本版最小规范只约束“DSL 结构”，不强行锁死提取引擎实现。

建议：

- HTML 页面常用 CSS 选择器
- XML 场景可用 XPath

如需更细的 `selector_type`、`jsonpath`、`regex`、`ai`、`ocr`，可以在后续版本继续补，但不影响本版主骨架。

---

## 05.12 `bind`

`bind` 用于计算中间变量，不直接发请求，也不直接输出结果。

```yaml
bind:
  period_date:
    from: "$fields.period_date"

  year:
    from: "$bind.period_date"
    transforms:
      - type: "date_format"
        input_format: "%Y-%m-%d"
        format: "%Y"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `bind.<name>` | value | 是 | 无 | 一个命名中间变量，值模型复用统一 `value` 语法 |

说明：

- 左侧字段名完全由模板作者自定义。
- `bind` 结果通过 `$bind.xxx` 引用。
- 适合做日期拆分、字符串拼接、字段兜底等中间计算。

---

## 05.13 统一值模型

### 05.13.1 基本形式

一个值支持以下几种写法：

字面量：

```yaml
title: "新闻标题"
timeout: 10000
```

`from`：

```yaml
title:
  from: "$meta.title"
```

`template + vars`：

```yaml
url:
  template: "https://example.com/{id}/{page}"
  vars:
    id:
      from: "$bind.id"
    page: 1
```

`selector`：

```yaml
title:
  selector: "h1"
  text: true
```

带 `transforms` 和 `fallback`：

```yaml
title:
  from: "$fields.title"
  transforms:
    - type: "trim"
  fallback:
    from: "$meta.title"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `literal` | string / number / bool / object / array | 否 | 无 | 直接写常量值 |
| `from` | string | 否（与 `template / selector` 互斥） | 无 | 从上下文中取值 |
| `template` | string | 否（与 `from / selector` 互斥） | 无 | 模板字符串 |
| `selector` | string | 否（与 `from / template` 互斥） | 无 | 从当前页面或当前 `follow.item` 作用域中提取值 |
| `text` | bool | 否 | `false` | 配合 `selector` 表示取文本 |
| `html` | bool | 否 | `false` | 配合 `selector` 表示取 HTML |
| `attr` | string | 否 | 无 | 配合 `selector` 表示取属性 |
| `vars` | object | 否 | `{}` | 模板变量集合 |
| `transforms` | array<object> | 否 | `[]` | 对当前值按顺序做加工 |
| `fallback` | value | 否 | 无 | 主值为空或取不到时的兜底值 |

### 05.13.2 字段说明

- `from`
  从上下文中取值。
- `template`
  模板字符串。
- `selector`
  从当前页面，或当前 `follow.item` 的节点作用域中提取值。
- `vars`
  模板变量，变量值本身也遵循统一值模型。
- `transforms`
  对当前值按顺序做加工。
- `fallback`
  当前值为空或取不到时的兜底值。

补充说明：

- `from / template / selector` 三者互斥。
- `vars` 只在 `template` 场景下生效。
- `selector` 场景下，`text / html / attr` 一次最多写一种。
- `selector` 如果不写 `html / attr`，当前实现默认按文本提取。
- `object / array` 字面量当前按“常量值”处理，不会递归解释其内部的 `from / template / selector`。
- 如果要在结构化对象里继续写值表达式，这个能力后续再单独设计；当前 v1 不支持。

### 05.13.3 推荐上下文前缀

- `$now`
- `$env.xxx`
- `$fields.xxx`
- `$bind.xxx`
- `$meta.xxx`
- `$request.url`
- `$request.method`
- `$response.url`
- `$response.status`

### 05.13.4 当前支持的 `transforms`

- `trim`
  去首尾空白。
- `replace`
  替换字符串。
- `regex`
  正则提取或替换。
- `split`
  按分隔符拆分。
- `join`
  按分隔符拼接。
- `pick`
  从数组中取指定位置。
- `date_format`
  日期格式转换。
- `resolve_url`
  把相对路径补全为绝对 URL。

说明：

- 当前 `validate` 阶段会直接校验 `transforms[].type` 是否在这份支持列表里。
- 缺少关键参数的 transform 也会在 `validate` 阶段直接报错。

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `transforms[].type` | string | 是 | 无 | transform 类型，例如 `trim / replace / regex / date_format` |
| `transforms[].<arg>` | any | 否 | 无 | 各 transform 的私有参数，例如 `format / input_format / expr` |

---

## 05.14 `follow`

### 05.14.1 基本结构

```yaml
follow:
  - item: ".news-list li"
    next_step: "parse_detail"

    request:
      url:
        selector: "a"
        attr: "href"

    meta:
      title:
        selector: "a"
        attr: "title"

    allow_url_pattern:
      - "^https?://"

    engine:
      concurrency: "detail_serial"
      interval: "origin_guard"
      rate_limit: "origin_budget"
      retry_by_status: "default_http_retry"
      retry_by_error: "detail_error_retry"
      dedup: "request_url_with_edition"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `follow[].item` | string | 否 | 无 | 当前一条子链路对应哪个页面节点 |
| `follow[].next_step` | string | 是 | 无 | 子链路进入的下一个 step |
| `follow[].request` | object | 是 | 无 | 下一跳请求配置 |
| `follow[].meta` | object | 否 | 无 | 绑定到当前下一跳请求上的上下文 |
| `follow[].allow_url_pattern` | array<string> | 否 | 无 | URL 过滤规则 |
| `follow[].engine.concurrency` | string | 否 | 无 | 引用一条 `engine.concurrency` 配置 |
| `follow[].engine.interval` | string | 否 | 无 | 引用一条 `engine.interval` 配置 |
| `follow[].engine.rate_limit` | string | 否 | 无 | 引用一条 `engine.rate_limit` 配置 |
| `follow[].engine.auto_throttle` | string | 否 | 无 | 引用一条 `engine.auto_throttle` 配置 |
| `follow[].engine.retry_by_status` | string | 否 | 无 | 引用一条 `engine.retry_by_status` 配置 |
| `follow[].engine.retry_by_error` | string | 否 | 无 | 引用一条 `engine.retry_by_error` 配置 |
| `follow[].engine.dedup` | string | 否 | 无 | 当前 follow 额外挂一条去重规则 |

字段说明：

- `item`
  当前 follow 在页面中按哪个子节点生成子链路。
- `next_step`
  子链路进入的下一个 step。
- `request`
  下一跳请求配置。
- `meta`
  绑定到当前下一跳请求上的上下文。
- `allow_url_pattern`
  允许通过的 URL 正则列表。
- `engine.concurrency / engine.interval / engine.rate_limit / engine.auto_throttle`
  当前 follow 额外挂下载前 middleware 配置。
- `engine.retry_by_status / engine.retry_by_error`
  当前 follow 额外挂重试 middleware 配置。
- `engine.dedup`
  当前 follow 额外挂一条去重规则。

补充说明：

- `engine.*` 这里只引用顶层 `engine` 中已经注册好的规则名。
- `follow` 本身不内联具体限流 / 重试 / 去重参数。

### 05.14.2 `item` 是什么

`item` 不是 `for_each`，也不是数组循环语法。

它的作用可以直白理解成：

“这一条 follow，要先在页面里找到哪一类节点，然后每个节点各走一遍完整链路”。

例如：

```yaml
item: ".news-list li"
```

表示：

- 每个 `li` 对应一条子链路
- 引擎按每个 `li` 逐条执行一次完整流程
- 每条流程里都会产生自己的 `request + meta + next_step`
- 也就是每个命中的节点，都会变成一条独立的数据流转链路

如果当前页面只会产生一条下一跳请求，可以省略 `item`。

### 05.14.3 `meta` 的语义

`meta` 不是全局缓存，也不是 `url -> meta` 的大字典。

它表示：

“当前这一条下一跳请求自带的上下文”。

例如：

- 第一个详情页请求带 `title=t1`
- 第二个详情页请求带 `title=t2`

它们是两条不同的子链路，各自带各自的 `meta`。

进入下一步后，直接通过 `$meta.xxx` 使用。

### 05.14.4 作用顺序

当前 follow 生成请求时，建议固定顺序如下：

1. 先生成 `request.url`
2. 如果是相对 URL，则补全为绝对 URL
3. 应用 `request.query`
4. 执行 `allow_url_pattern`
5. 执行 `engine.dedup`
6. 请求入队
7. 在下载前执行 `engine.concurrency / engine.interval / engine.rate_limit / engine.auto_throttle`
8. 发请求
9. 在下载结果阶段执行 `engine.retry_by_status / engine.retry_by_error`
10. 成功后进入 `next_step`

---

## 05.15 `request`

### 05.15.1 完整模型

```yaml
request:
  mode: "http"
  method: "GET"
  url: ...
  query: ...
  headers: ...
  cookies: ...
  timeout: 10000
  proxy: ...
  session: ...
  encoding: ...
  priority: ...
  flags: ...
  cb_kwargs: ...
  errback: ...
  allow_redirects: true
  skip: ...
  body: ...
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `request.mode` | string | 否 | `http` | 下载模式，当前支持 `http / browser` |
| `request.method` | string | 否 | `GET` | HTTP 方法 |
| `request.url` | string / object | 是 | 无 | 请求地址 |
| `request.query` | object | 否 | `{}` | Query 参数 |
| `request.headers` | object | 否 | `{}` | 请求头 |
| `request.cookies` | object | 否 | `{}` | Cookie |
| `request.timeout` | number / object | 否 | 无 | 请求超时，单位固定 `ms`，也可写统一值表达式 |
| `request.proxy` | string / object | 否 | 无 | 代理配置 |
| `request.session` | string / object | 否 | 无 | session 标识，或统一值表达式 |
| `request.encoding` | string / object | 否 | 无 | 响应解码编码，或统一值表达式 |
| `request.priority` | number / object | 否 | 无 | 调度优先级，或统一值表达式 |
| `request.flags` | array<value> | 否 | `[]` | 当前请求附带的 flag 列表 |
| `request.cb_kwargs` | object | 否 | `{}` | 传给 callback 的参数映射 |
| `request.errback` | string | 否 | 无 | 下载失败时调用哪个 errback |
| `request.allow_redirects` | bool | 否 | 下载器默认值 | 是否允许自动跟随重定向 |
| `request.skip` | array<string> | 否 | `[]` | 显式跳过哪些 middleware |
| `request.body` | object | 否 | 无 | 请求体 |

字段说明：

- `mode`
  下载模式，当前支持 `http` 与 `browser`。
- `method`
  HTTP 方法，默认 `GET`。
- `url`
  请求地址。
- `query`
  Query 参数。
- `headers`
  请求头。
- `cookies`
  Cookie。
- `timeout`
  请求超时，单位固定 `ms`。
- `proxy`
  代理配置。
- `session`
  session 标识。
- `encoding`
  响应解码编码。
- `priority`
  调度优先级。
- `flags`
  当前请求附带的 flag 列表。
- `cb_kwargs`
  传给 callback 的参数映射。
- `errback`
  下载失败时调用哪个 errback。
- `allow_redirects`
  是否允许自动跟随重定向。
- `skip`
  显式跳过指定 middleware，例如 `["dedup"]`。
- `body`
  请求体。

说明：

- `request.dont_filter` 已移除。
- 如果要跳过去重，直接写 `request.skip: ["dedup"]`。

### 05.15.2 `url`

`url` 支持三类常见写法。

字面量：

```yaml
url: "https://example.com/a"
```

引用已有值：

```yaml
url:
  from: "$meta.detail_url"
```

模板拼接：

```yaml
url:
  template: "https://example.com/{year}/{id}.html"
  vars:
    year:
      from: "$bind.year"
    id:
      from: "$bind.id"
```

在 `follow.item` 场景下，`url` 还允许直接从当前 item 提取：

```yaml
url:
  selector: "a"
  attr: "href"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `request.url.from` | string | 否（与其它 URL 生成方式互斥） | 无 | 从已有上下文引用 URL |
| `request.url.template` | string | 否（与其它 URL 生成方式互斥） | 无 | 模板生成 URL |
| `request.url.vars` | object | 否 | `{}` | 模板变量 |
| `request.url.selector` | string | 否（通常用于 `follow.item` 场景） | 无 | 从当前 item 节点内提取 URL |
| `request.url.attr` | string | 否 | 无 | 当使用 `selector` 时取哪个属性 |

这时语义是：

- 当前 item 节点内找到 `a`
- 取其 `href`
- 当前 item 产生一条子链路

补充说明：

- `request.url` 的对象形态只表示统一值表达式。
- 也就是只能写 `from / template / selector / transforms / fallback` 这一套键。
- 不支持任意对象字面量，例如 `{ value: "https://example.com" }`。

### 05.15.3 `query / headers / cookies`

这三个字段都建议使用统一值模型。

例如：

```yaml
query:
  page: 1
  source:
    from: "$meta.source"

headers:
  referer:
    from: "$response.url"

cookies:
  sid:
    from: "$meta.sid"
```

`session / encoding / priority / flags / cb_kwargs` 也沿用同一套值模型。

例如：

```yaml
session:
  from: "$meta.session_id"

encoding: "utf-8"

priority:
  from: "$meta.priority"

flags:
  - "detail"
  - from: "$meta.channel"

cb_kwargs:
  channel:
    from: "$meta.channel"
```

### 05.15.4 `timeout`

`timeout` 单位固定为 `ms`。

它也支持统一值表达式，例如：

```yaml
timeout:
  from: "$meta.timeout_ms"
```

约定：

- 字段名不再写成 `timeout_ms`
- 统一在文档层约定时间单位

同类字段还包括：

- `interval`
- `backoff`
- `start_interval`
- `min_interval`
- `max_interval`

### 05.15.5 `proxy / session / encoding / priority`

最小可支持两种形式：

直接写代理地址：

```yaml
proxy: "http://user:pass@ip:port"
```

从上下文引用已有代理地址：

```yaml
proxy:
  from: "$meta.proxy_url"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `request.proxy` | string / object | 否 | 无 | 代理地址，或统一值表达式 |
| `request.proxy.from` | string | 否 | 无 | 从已有上下文引用代理地址 |
| `request.session` | string / object | 否 | 无 | session 标识，或统一值表达式 |
| `request.encoding` | string / object | 否 | 无 | 解码编码，或统一值表达式 |
| `request.priority` | number / object | 否 | 无 | 调度优先级，或统一值表达式 |

说明：

- 当前 `proxy` 走统一值模型，所以也可以继续使用 `template / transforms / fallback` 这些表达式能力。
- 当前不支持 `request.proxy.ref` 这类代理注册表引用写法。
- `session / encoding / priority` 也都走同一套统一值模型。
- 这些字段的对象形态也只表示统一值表达式，不支持任意对象字面量。

### 05.15.6 `errback / allow_redirects`

```yaml
request:
  errback: "handle_detail_error"
  allow_redirects: false
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `request.errback` | string | 否 | 无 | 下载失败时调用哪个 errback |
| `request.allow_redirects` | bool | 否 | 下载器默认值 | 是否允许自动跟随重定向 |

说明：

- `errback` 这里只写回调名字。
- `allow_redirects` 当前是布尔值，不走值表达式。

### 05.15.7 `body`

v1 建议只支持三种：

```yaml
body:
  json:
    keyword: "test"
```

```yaml
body:
  form:
    keyword: "test"
```

```yaml
body:
  raw: "{\"keyword\":\"test\"}"
```

```yaml
body:
  raw:
    from: "$meta.raw_body"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `request.body.json` | object | 否（三选一） | 无 | JSON 请求体 |
| `request.body.form` | object | 否（三选一） | 无 | Form 请求体 |
| `request.body.raw` | string / object | 否（三选一） | 无 | 原始请求体，或统一值表达式 |

约束：

- `json / form / raw` 三选一
- `GET` 通常不写 `body`
- `body.raw` 的对象形态只表示统一值表达式，不支持任意对象字面量

### 05.15.8 `skip`

`skip` 用来在 request 级别显式跳过某些 middleware。

例如：

```yaml
request:
  url: "https://example.com/detail/1"
  skip:
    - "dedup"
```

```yaml
request:
  url: "https://example.com/detail/1"
  skip:
    - "retry_by_status"
    - "retry_by_error"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `request.skip` | array<string> | 否 | `[]` | 当前请求要跳过的 middleware 名称 |

说明：

- `skip` 中直接写 middleware 名称。
- 内置中间件例如：`dedup`、`concurrency`、`interval`、`rate_limit`、`auto_throttle`、`retry_by_status`、`retry_by_error`。
- 如果有自定义 middleware，也可以直接写自定义导出的名称。

---

## 05.16 `output`

### 05.16.1 基本结构

```yaml
output:
  item: ...
  validate: ...
  sinks:
    - "article_db"
    - "article_file"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `output.item` | object | 是 | 无 | 最终结果字段映射 |
| `output.validate` | object | 否 | 无 | 最终结果校验规则 |
| `output.sinks` | array<string> | 是 | 无 | 最终结果发往哪些 sink |

字段说明：

- `item`
  最终结果字段映射。
- `validate`
  最终结果校验规则。
- `sinks`
  最终结果要输出到哪些 sink。

### 05.16.2 `item`

`item` 的每个字段都使用统一值模型。

例如：

```yaml
item:
  title:
    from: "$meta.title"
    fallback:
      from: "$fields.title"

  content:
    from: "$fields.content"

  source_url:
    from: "$response.url"
```

这表示：

- 先优先使用列表页传下来的 `meta.title`
- 没有时再用详情页解析出的 `fields.title`
- 最终组装成一条输出结果

### 05.16.3 `sinks`

```yaml
output:
  sinks:
    - "article_db"
    - "article_file"
```

说明：

- `output.sinks` 只是引用顶层 `sinks` 注册表中的名字。
- 可以同时输出到多个 sink。
- 同一条结果会 fan-out 到所有目标 sink。

当前实现说明：

- 当前版本会校验 `output.sinks` 是否引用了已声明的顶层 `sinks`。
- 运行时会把 `output.sinks` 解析为目标 store 实例，并在统一 `pipeline -> store` 主链末端 fan-out 到目标 store。
- 顶层 `sinks` 注册表本身不会自动创建 store 实例；业务侧仍需用 `Engine::with_store(...)` / `Engine::with_stores(...)` 注册真实 store。

---

## 05.17 `validate`

### 05.17.1 内联校验

校验直接写在 `output.validate` 中。

这样更适合当前 DSL 的单链路输出模型：

- 大多数任务只有一种输出结构
- 直接把校验写在 `output.validate` 更直观
- 可以减少命名 schema 带来的额外心智负担

### 05.17.2 基本结构

```yaml
validate:
  required:
    - "title"
    - "content"
    - "source_url"

  fields:
    title:
      type: "string"
      min_length: 1
      max_length: 120

    content:
      type: "string"
      min_length: 20

    source_url:
      type: "string"
      format: "url"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `validate.required` | array<string> | 否 | `[]` | 哪些字段必须存在 |
| `validate.fields` | object | 否 | `{}` | 字段级校验规则 |

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `validate.fields.<name>.type` | string | 是（若声明该字段） | 无 | 字段类型 |
| `validate.fields.<name>.min_length` | int | 否 | 无 | 最小长度，要求非负整数 |
| `validate.fields.<name>.max_length` | int | 否 | 无 | 最大长度，要求非负整数 |
| `validate.fields.<name>.format` | string | 否 | 无 | 特定格式，例如 `url` |
| `validate.fields.<name>.pattern` | string | 否 | 无 | 正则表达式 |
| `validate.fields.<name>.enum` | array<any> | 否 | 无 | 枚举值列表 |

字段说明：

- `required`
  哪些字段必须存在。
- `fields.<name>.type`
  字段类型。
- `min_length`
  最小长度。
- `max_length`
  最大长度。
- `format`
  特定格式，例如 `url`。
- `pattern`
  正则表达式。
- `enum`
  枚举值列表。

补充说明：

- `format` 当前只支持 `url / datetime / date`。
- `min_length / max_length` 当前要求写非负整数。

### 05.17.3 推荐支持的校验规则

- `type`
- `min_length`
- `max_length`
- `format`
- `pattern`
- `enum`

常见示例：

```yaml
validate:
  required:
    - "title"

  fields:
    title:
      type: "string"
      min_length: 1
      max_length: 120

    channel:
      type: "string"
      enum: ["news", "finance", "policy"]
```

### 05.17.4 代码校验

本版 DSL 只保留声明式校验：

- `required`
- `fields`

`output.validate.rule` 已从当前 DSL 中移除，不再作为支持字段。

---

## 05.18 `allow_url_pattern`

```yaml
allow_url_pattern:
  - "^https://example\\.com/detail/.+\\.html$"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `allow_url_pattern` | array<string> | 否 | 无 | 当前 seed 或 follow 允许放行的 URL 正则列表 |

说明：

- 这是请求层过滤规则。
- 只对当前 seed 或 follow 生成的 URL 生效。
- pattern 建议按正则处理。
- 任一 pattern 命中即可放行。
- 全部未命中则当前请求丢弃。

推荐处理时机：

- 在 URL 生成并规范化之后
- 在 `engine.dedup` 之前

---

## 05.19 执行顺序

### 05.19.1 step 内部

建议固定顺序：

1. 解析当前页面 `fields`
2. 计算当前页面 `bind`
3. 生成 `follow` 的下一跳请求，或执行 `output`

### 05.19.2 follow 生成请求

建议固定顺序：

1. 确定当前 `item` 作用域
2. 生成当前子链路的 `request.url`
3. 生成当前子链路的 `meta`
4. 如果是相对 URL，则补全
5. 执行 `allow_url_pattern`
6. 执行 `engine.dedup`
7. 请求入队
8. 在下载前执行 `engine.concurrency / engine.interval / engine.rate_limit / engine.auto_throttle`
9. 发请求
10. 在下载结果阶段执行 `engine.retry_by_status / engine.retry_by_error`
11. 成功后进入 `next_step`

### 05.19.3 output 输出结果

建议固定顺序：

1. 组装 `output.item`
2. 执行 `output.validate`
3. 校验通过后按 `output.sinks` 从 engine store 注册表解析目标 store 实例
4. item 进入统一 `pipeline -> store` 主链，并 fan-out 到目标 store
5. 校验失败则丢弃当前结果，并记录失败原因

---

## 05.20 小结

本版 DSL v1 的核心骨架已经固定为：

- 顶层注册表：`engine(dedup / concurrency / interval / rate_limit / auto_throttle / retry_by_status / retry_by_error) / sinks`
- 链路起点：`seeds`
- 页面处理：`steps`
- 页面解析：`fields`
- 中间变量：`bind`
- 下一跳请求：`follow`
- 链路上下文：`meta`
- 最终输出：`output`

这套结构的目标不是暴露运行时细节，而是把“一条数据怎么走出来”描述清楚。
