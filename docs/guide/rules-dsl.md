# 05 Rules DSL 设计（v1）

本章给出新的 rules DSL v1 设计稿。

本版 DSL 不再沿用旧的 `parse / next_url_config / schemas` 那套结构，而是改为更直观的“单链路流转模型”：

- 从 `seed` 开始发起一条链路
- 链路进入某个 `step`
- `step` 可以继续 `follow` 生成下一跳子链路
- 也可以在 `output` 处产出最终结果

本版重点是把以下几类能力拆清楚：

- 请求层：`request / schedule / retry / dedup / allow_url_pattern`
- 页面解析层：`fields / bind`
- 链路上下文：`meta`
- 结果输出层：`output / validate / sinks`

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
- `schedule`
- `retry`
- `dedup`
- `allow_url_pattern`

结果相关能力放在输出层：

- `output.item`
- `output.validate`
- `output.sinks`

### 05.1.4 配置集中定义、局部单值引用

以下三类规则统一采用“顶层注册表 + 局部单值引用”的方式：

- `schedule`
- `retry`
- `dedup`

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

schedule:
  limits: ...

retry:
  ...

dedup:
  ...

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
| `schedule` | object | 否 | 无 | 限流规则注册表 |
| `retry` | object | 否 | 无 | 重试规则注册表 |
| `dedup` | object | 否 | 无 | 请求去重规则注册表 |
| `sinks` | object | 否 | 无 | 输出目标注册表 |
| `seeds` | array<object> | 是 | 无 | 起始请求列表 |
| `steps` | array<object> | 是 | 无 | 页面处理步骤列表 |

顶层块含义：

- `spider`
  爬虫级基础配置。
- `schedule`
  限流规则注册表。
- `retry`
  重试规则注册表。
- `dedup`
  请求去重规则注册表。
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

schedule:
  limits:
    spider_global:
      key: "spider"
      inherit: true
      concurrency: 5
      interval: 300

    origin_guard:
      key: "origin"
      concurrency: 2
      interval: 800

    detail_slow:
      key: "origin"
      concurrency: 1
      interval: 1500

retry:
  default_retry:
    inherit: true
    count: 3
    http_status: [429, 500, 502, 503, 504]
    backoff: [1000, 3000, 5000]

  detail_retry:
    count: 4
    http_status: [429, 500, 502, 503, 504]
    backoff: [1000, 3000, 5000, 8000]

dedup:
  request_url:
    inherit: true
    key:
      - "$request.url"
    ttl: 604800000

  request_url_with_edition:
    key:
      - "$request.url"
      - "$meta.edition_id"
    ttl: 604800000

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
      latest_node:
        selector: "//period[last()]"
        kind: "node"

    bind:
      period_date:
        from: "$fields.latest_node"
        transforms:
          - type: "xpath_text"
            expr: "./@date"

      edition_id:
        from: "$fields.latest_node"
        transforms:
          - type: "xpath_text"
            expr: "./@id"

      page_no:
        from: "$fields.latest_node"
        transforms:
          - type: "xpath_text"
            expr: "./@number"

      year:
        from: "$bind.period_date"
        transforms:
          - type: "date_format"
            input_format: "%Y-%m-%d"
            format: "%Y"

      month:
        from: "$bind.period_date"
        transforms:
          - type: "date_format"
            input_format: "%Y-%m-%d"
            format: "%m"

      day:
        from: "$bind.period_date"
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
            from: "$bind.period_date"
          edition_id:
            from: "$bind.edition_id"
          page_no:
            from: "$bind.page_no"

        dedup:
          rule: "request_url"

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

        schedule:
          limit: "detail_slow"

        retry:
          rule: "detail_retry"

        dedup:
          rule: "request_url_with_edition"

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
- 不把 `retry / dedup / sinks / steps` 混到这里。

---

## 05.5 `schedule`

### 05.5.1 顶层结构

```yaml
schedule:
  limits:
    spider_global:
      key: "spider"
      inherit: true
      concurrency: 5
      interval: 300

    detail_slow:
      key: "origin"
      concurrency: 1
      interval: 1500
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `schedule.limits` | object | 是（若启用 `schedule`） | 无 | 限流规则注册表 |
| `schedule.limits.<name>.key` | string | 是 | 无 | 限流规则按什么维度分桶生效 |
| `schedule.limits.<name>.inherit` | bool | 否 | `false` | 是否全局默认生效 |
| `schedule.limits.<name>.concurrency` | int | 否（与 `interval` 至少一项） | 无 | 最大并发数 |
| `schedule.limits.<name>.interval` | number | 否（与 `concurrency` 至少一项） | 无 | 最小调度间隔，单位固定 `ms` |

字段说明：

- `limits`
  限流规则注册表。
- `limits.<name>`
  一条命名限流规则。
- `key`
  这条限流规则按什么维度生效。
- `inherit`
  是否全局默认生效。
- `concurrency`
  最大并发数。
- `interval`
  最小调度间隔，单位固定 `ms`。

### 05.5.2 `key` 是什么

`key` 表示“这条限流规则，是按什么维度分桶生效”。

v1 推荐固定支持：

- `spider`
  整个 spider 共用一个限流桶。
- `origin`
  同一个源站共用一个限流桶，通常指 `scheme + host + port`。

例如：

- `key: "spider"`
  代表整个任务统一受这条规则限制。
- `key: "origin"`
  代表同域请求受这条规则限制。

### 05.5.3 `inherit`

- `inherit: true`
  这条规则默认全局生效。
- 不写 `inherit`
  默认是 `false`，只在局部显式引用时生效。

### 05.5.4 局部引用

`seed` 和 `follow` 都可以额外挂一条限流规则：

```yaml
follow:
  - next_step: "parse_detail"
    schedule:
      limit: "detail_slow"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `seed.schedule.limit` | string | 否 | 无 | 当前 seed 额外挂一条限流规则 |
| `follow.schedule.limit` | string | 否 | 无 | 当前 follow 额外挂一条限流规则 |

约束：

- `limit` 只能是单值字符串。
- `limit` 不支持数组。
- 实际生效规则 = 所有 `inherit: true` 的规则 + 当前局部 `limit`。

设计原因：

- 用户不用处理多规则组合的歧义。
- 每个步骤最多只补一条特殊限制，配置边界更清晰。

---

## 05.6 `retry`

### 05.6.1 顶层结构

```yaml
retry:
  default_retry:
    inherit: true
    count: 3
    http_status: [429, 500, 502, 503, 504]
    backoff: [1000, 3000, 5000]

  detail_retry:
    count: 4
    http_status: [429, 500, 502, 503, 504]
    backoff: [1000, 3000, 5000, 8000]
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `retry.<name>.inherit` | bool | 否 | `false` | 是否全局默认生效 |
| `retry.<name>.count` | int | 是 | 无 | 最大重试次数 |
| `retry.<name>.http_status` | array<int> | 否 | 无 | 哪些状态码触发重试 |
| `retry.<name>.backoff` | array<number> | 否 | 无 | 每次重试前等待多久，单位固定 `ms` |

字段说明：

- `retry.<name>`
  一条命名重试规则。
- `inherit`
  是否全局默认生效。
- `count`
  最大重试次数。
- `http_status`
  哪些状态码触发重试。
- `backoff`
  每次重试前的等待时间列表，单位固定 `ms`。

### 05.6.2 局部引用

```yaml
follow:
  - next_step: "parse_detail"
    retry:
      rule: "detail_retry"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `seed.retry.rule` | string | 否 | 无 | 当前 seed 额外挂一条重试规则 |
| `follow.retry.rule` | string | 否 | 无 | 当前 follow 额外挂一条重试规则 |

约束：

- `rule` 只能是单值字符串。
- `rule` 不支持数组。
- 实际生效规则 = 所有 `inherit: true` 的规则 + 当前局部 `rule`。

### 05.6.3 `backoff` 规则

- `count` 表示最多重试几次。
- `backoff` 可以比 `count` 短。
- 如果 `count` 大于 `backoff` 长度，超出的重试继续使用最后一个 `backoff` 值。

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

## 05.7 `dedup`

### 05.7.1 顶层结构

```yaml
dedup:
  request_url:
    inherit: true
    key:
      - "$request.url"
    ttl: 604800000

  request_url_with_edition:
    key:
      - "$request.url"
      - "$meta.edition_id"
    ttl: 604800000
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `dedup.<name>.inherit` | bool | 否 | `false` | 是否全局默认生效 |
| `dedup.<name>.key` | array<string> | 是 | 无 | 去重键组成字段列表 |
| `dedup.<name>.ttl` | number | 是 | 无 | 去重有效期，单位固定 `ms` |

字段说明：

- `dedup.<name>`
  一条命名去重规则。
- `inherit`
  是否全局默认生效。
- `key`
  去重键由哪些字段组成。
- `ttl`
  去重有效期，单位固定 `ms`。

### 05.7.2 局部引用

```yaml
follow:
  - next_step: "parse_detail"
    dedup:
      rule: "request_url_with_edition"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `seed.dedup.rule` | string | 否 | 无 | 当前 seed 额外挂一条去重规则 |
| `follow.dedup.rule` | string | 否 | 无 | 当前 follow 额外挂一条去重规则 |

约束：

- `rule` 只能是单值字符串。
- `rule` 不支持数组。
- 实际生效规则 = 所有 `inherit: true` 的规则 + 当前局部 `rule`。

### 05.7.3 `key` 怎么算

`key` 中的值按顺序取出，然后拼成最终的去重值。

例如：

```yaml
key:
  - "$request.url"
  - "$meta.edition_id"
```

则最终可规范化为：

```text
{request.url}|{meta.edition_id}
```

推荐支持的引用来源：

- `$request.url`
- `$request.method`
- `$meta.xxx`

规则建议：

- 任一 key 值缺失或为空时，当前请求直接丢弃，并记录 `invalid_dedup_key`。
- 拼接顺序必须严格按 `key` 数组顺序执行。

### 05.7.4 作用边界

本版 `dedup` 只做请求层去重，不做结果层去重。

也就是说：

- 它作用于“这条请求要不要继续发”
- 不作用于“最终 output.item 要不要重复写入”

结果层幂等由 sink 自己负责，例如数据库 `upsert`。

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

    schedule:
      limit: "detail_slow"

    retry:
      rule: "detail_retry"

    dedup:
      rule: "request_url"

    next_step: "parse_period_xml"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `seeds[].id` | string | 是 | 无 | seed 唯一标识 |
| `seeds[].request` | object | 是 | 无 | 起始请求配置 |
| `seeds[].meta` | object | 否 | 无 | 初始链路上下文 |
| `seeds[].allow_url_pattern` | array<string> | 否 | 无 | URL 过滤规则 |
| `seeds[].schedule.limit` | string | 否 | 无 | 当前 seed 额外挂一条限流规则 |
| `seeds[].retry.rule` | string | 否 | 无 | 当前 seed 额外挂一条重试规则 |
| `seeds[].dedup.rule` | string | 否 | 无 | 当前 seed 额外挂一条去重规则 |
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
- `schedule.limit`
  额外挂一条限流规则，可选。
- `retry.rule`
  额外挂一条重试规则，可选。
- `dedup.rule`
  额外挂一条去重规则，可选。
- `next_step`
  请求成功后进入哪个 step。

说明：

- seed 本质上也是一条请求。
- 所以它和 follow 一样，可以挂请求层能力。

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
- `step` 自己不放 `schedule / retry / dedup` 的具体参数。
- 请求控制只放在 `seed / follow` 上。

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

  node_ref:
    selector: "//period[last()]"
    kind: "node"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `fields.<name>.selector` | string | 是 | 无 | 当前页面中的提取表达式 |
| `fields.<name>.text` | bool | 否 | `false` | 取文本 |
| `fields.<name>.html` | bool | 否 | `false` | 取 HTML |
| `fields.<name>.attr` | string | 否 | 无 | 取指定属性 |
| `fields.<name>.kind` | string | 否 | 无 | 特殊提取模式，例如 `node` |

字段说明：

- `selector`
  当前页面中的提取表达式。
- `text: true`
  取文本。
- `html: true`
  取 HTML。
- `attr: "<name>"`
  取指定属性。
- `kind: "node"`
  直接保留节点对象，供后续 transform 使用。

约束：

- `text / html / attr / kind` 建议一次只写一种。
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
    from: "$fields.latest_node"
    transforms:
      - type: "xpath_text"
        expr: "./@date"

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
| `from` | string | 否（与 `template` 二选一） | 无 | 从上下文中取值 |
| `template` | string | 否（与 `from` 二选一） | 无 | 模板字符串 |
| `vars` | object | 否 | `{}` | 模板变量集合 |
| `transforms` | array<object> | 否 | `[]` | 对当前值按顺序做加工 |
| `fallback` | value | 否 | 无 | 主值为空或取不到时的兜底值 |

### 05.13.2 字段说明

- `from`
  从上下文中取值。
- `template`
  模板字符串。
- `vars`
  模板变量，变量值本身也遵循统一值模型。
- `transforms`
  对当前值按顺序做加工。
- `fallback`
  当前值为空或取不到时的兜底值。

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

### 05.13.4 推荐内置 `transforms`

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
- `xpath_text`
  从 node 中按 XPath 取值。
- `resolve_url`
  把相对路径补全为绝对 URL。

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

    schedule:
      limit: "detail_slow"

    retry:
      rule: "detail_retry"

    dedup:
      rule: "request_url_with_edition"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `follow[].item` | string | 否 | 无 | 当前 follow 的单条子链路节点范围 |
| `follow[].next_step` | string | 是 | 无 | 子链路进入的下一个 step |
| `follow[].request` | object | 是 | 无 | 下一跳请求配置 |
| `follow[].meta` | object | 否 | 无 | 绑定到当前下一跳请求上的上下文 |
| `follow[].allow_url_pattern` | array<string> | 否 | 无 | URL 过滤规则 |
| `follow[].schedule.limit` | string | 否 | 无 | 当前 follow 额外挂一条限流规则 |
| `follow[].retry.rule` | string | 否 | 无 | 当前 follow 额外挂一条重试规则 |
| `follow[].dedup.rule` | string | 否 | 无 | 当前 follow 额外挂一条去重规则 |

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
- `schedule.limit`
  当前 follow 额外挂一条限流规则。
- `retry.rule`
  当前 follow 额外挂一条重试规则。
- `dedup.rule`
  当前 follow 额外挂一条去重规则。

### 05.14.2 `item` 是什么

`item` 不是 `for_each`，也不是数组循环语法。

它的作用只有一个：

定义“当前 follow 的单条子链路，是从页面里的哪个子节点产生出来的”。

例如：

```yaml
item: ".news-list li"
```

表示：

- 每个 `li` 对应一条子链路
- 引擎按每个 `li` 逐条执行一次完整流程
- 每条流程里都会产生自己的 `request + meta + next_step`

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
5. 执行 `dedup`
6. 进入 `schedule`
7. 发请求
8. 失败时按 `retry` 处理
9. 成功后进入 `next_step`

---

## 05.15 `request`

### 05.15.1 完整模型

```yaml
request:
  method: "GET"
  url: ...
  query: ...
  headers: ...
  cookies: ...
  timeout: 10000
  proxy: ...
  body: ...
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `request.method` | string | 否 | `GET` | HTTP 方法 |
| `request.url` | string / object | 是 | 无 | 请求地址 |
| `request.query` | object | 否 | `{}` | Query 参数 |
| `request.headers` | object | 否 | `{}` | 请求头 |
| `request.cookies` | object | 否 | `{}` | Cookie |
| `request.timeout` | number | 否 | 无 | 请求超时，单位固定 `ms` |
| `request.proxy` | string / object | 否 | 无 | 代理配置 |
| `request.body` | object | 否 | 无 | 请求体 |

字段说明：

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
- `body`
  请求体。

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

### 05.15.4 `timeout`

`timeout` 单位固定为 `ms`。

约定：

- 字段名不再写成 `timeout_ms`
- 统一在文档层约定时间单位

同类字段还包括：

- `interval`
- `backoff`
- `ttl`

### 05.15.5 `proxy`

最小可支持两种形式：

直接写代理地址：

```yaml
proxy: "http://user:pass@ip:port"
```

引用运行时已有代理配置：

```yaml
proxy:
  ref: "default_proxy"
```

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `request.proxy` | string | 否 | 无 | 直接写代理地址 |
| `request.proxy.ref` | string | 否 | 无 | 引用运行时已有代理配置 |

### 05.15.6 `body`

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

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `request.body.json` | object | 否（三选一） | 无 | JSON 请求体 |
| `request.body.form` | object | 否（三选一） | 无 | Form 请求体 |
| `request.body.raw` | string | 否（三选一） | 无 | 原始请求体 |

约束：

- `json / form / raw` 三选一
- `GET` 通常不写 `body`

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

---

## 05.17 `validate`

### 05.17.1 内联校验

v1 不单独引入顶层 `schemas`。

原因：

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
| `validate.rule` | string | 否 | 无 | 引用运行时已有代码校验器 |

| 字段 | 类型 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- | --- |
| `validate.fields.<name>.type` | string | 是（若声明该字段） | 无 | 字段类型 |
| `validate.fields.<name>.min_length` | number | 否 | 无 | 最小长度 |
| `validate.fields.<name>.max_length` | number | 否 | 无 | 最大长度 |
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

如果运行时已经支持代码校验，可以在 `validate` 中加一个轻量引用：

```yaml
validate:
  required:
    - "title"
    - "content"

  fields:
    title:
      type: "string"
      min_length: 1

  rule: "article_guard"
```

其中：

- `rule`
  引用运行时预注册的代码校验器

用途：

- 适合做声明式规则不方便表达的复杂校验
- 例如两个字段互斥、时间先后关系、跨字段一致性等

说明：

- `rule` 的注册方式属于运行时实现，不由 DSL 本身管理
- DSL 只负责引用名字

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
- 在 dedup 之前

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
6. 执行 `dedup`
7. 进入 `schedule`
8. 发请求
9. 失败时按 `retry` 处理
10. 成功后进入 `next_step`

### 05.19.3 output 输出结果

建议固定顺序：

1. 组装 `output.item`
2. 执行 `output.validate`
3. 校验通过后写入 `output.sinks`
4. 校验失败则丢弃当前结果，并记录失败原因

---

## 05.20 本版明确不纳入的旧概念

为了保证 v1 简洁，本版明确不把以下概念作为核心 DSL 结构：

- `parse` 大对象
- `next_url_config`
- `collections`
- `for_each`
- 顶层 `schemas`

原因如下：

- `fields` 已经承担当前页面解析职责，不需要再包一层 `parse`
- `follow.request.url` 已经承担下一跳生成职责，不需要单独的 `next_url_config`
- `item` 已经能表达“当前子链路从哪个节点产生”，不需要再引入 `collections / for_each`
- 一个任务通常只有一种输出结构，v1 直接使用 `output.validate` 即可

---

## 05.21 小结

本版 DSL v1 的核心骨架已经固定为：

- 顶层注册表：`schedule / retry / dedup / sinks`
- 链路起点：`seeds`
- 页面处理：`steps`
- 页面解析：`fields`
- 中间变量：`bind`
- 下一跳请求：`follow`
- 链路上下文：`meta`
- 最终输出：`output`

这套结构的目标不是暴露运行时细节，而是把“一条数据怎么走出来”描述清楚。
