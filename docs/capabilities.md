# 功能说明

这份文档按功能模块整理当前 `kun` 已实现的底层能力、运行边界和主要命名含义。
README 负责总览，这里负责把每个模块现在到底能做什么、还缺什么讲清楚。

## 阅读方式

如果你是第一次看这个项目，推荐按这个顺序理解：

- 先看 `Request` / `Download` / `Response`，理解“请求如何发出、结果如何返回”
- 再看 `Scheduler` / `Pipeline` / `Store`，理解“任务如何流转、item 如何处理、最终如何落地”
- 最后看 `Validation` / `Plugins` / `DSL 当前定位`，理解扩展边界和后续方向

## Request

`Request` 是统一的执行单元。

- 代码爬虫里手写的请求，和后续 DSL 生成的请求，本质上都应该落成同一个 `Request`
- 当前已经接线的共享请求语义包括：`method`、`headers`、`body`、`timeout`、`proxy`、`session`、request cookies
- `meta` 是请求级上下文参数，用来挂当前请求和后续链路要透传的数据；它更接近 Scrapy 的函数参数/上下文，而不是框架私自塞内部控制字段的地方
- `follow()` 会继承请求级共享语义，再按子请求显式覆盖

这部分的设计目标是：不管请求最后走 HTTP 还是 browser，入口模型都尽量一致。

## Download

当前下载能力分成两类实现，但它们都只是 `Request` 的不同执行方式，不是两套互相割裂的运行时。

### `download::Http`

- 基于 `reqwest`
- 已接线真实的 timeout、proxy、redirect、cookie jar、session cookies
- `Response.body` 保存原始字节
- `Response.text` 从 `body` 按统一解码规则派生

### `download::Browser`

- 当前走 `playwright-rs`
- 用于打开页面、执行浏览器导航、拿渲染后的 HTML
- 已支持最小的 `method`、`body`、`headers`、`timeout`、`proxy`、request cookies、session
- 已支持 `wait_for` 这类页面就绪等待配置，用于在取 HTML 前等待目标内容出现
- 已支持内置 `fingerprint_profile = desktop_zh_cn | desktop_en_us`
- 已支持最小 `stealth = true` bootstrap，覆盖 `navigator.webdriver`、`navigator.languages`、`navigator.platform`、最小 `window.chrome` 与 notifications permissions 查询补丁
- 同一个 browser session 会复用稳定的 user data dir，并做最小串行化，避免并发抢 profile

当前仍未收敛、并会继续显式报错或保留空白的部分：

- 自定义 `fingerprint_profile` 名称
- 更完整的第三方 stealth 套件或更高阶浏览器指纹伪装能力
- `ip_address`、`certificate` 这类 Playwright 当前接口拿不到的响应侧字段

这部分的设计目标是：HTTP 和 browser 只是两种下载方式，最终都回到统一请求语义。
browser 在这里的角色是“渲染型下载器”，不是另起一套通用浏览器自动化 runtime；当前只保留导航、等待页面就绪和返回最终 HTML 这类爬虫抓取直接需要的能力。

## Response

`Response` 是下载结果的统一表示。

- `body` 是原始响应字节
- `text` 是从 `body` 解码出来的字符串视图
- 当前文本解码优先顺序是：BOM -> `Content-Type charset` -> 文档内编码声明 -> apparent encoding 猜测 -> UTF-8 lossy
- HTTP 和 browser 最终都要回到统一的 `Response` 语义上，方便 parser、callback、pipeline、store 复用

## Scheduler

`scheduler` 负责管理“已发现但尚未完成”的任务流转。

当前内置实现是 `scheduler::Memory`，它把任务分成三组状态：

- `ready`：已经可以立即执行的任务
- `delayed`：因为 delay / retry backoff 等原因，还没到执行时间的任务
- `inflight`：已经取出执行、正在运行、尚未完成或重新入队的任务

当前对外更推荐直接使用 `scheduler` 根导出的类型：

- `scheduler::Memory`
- `scheduler::Redis`
- `scheduler::checkpoint::File`
- `scheduler::checkpoint::Redis`
- `scheduler::Task`
- `scheduler::TaskId`
- `scheduler::Scheduler`

### `scheduler` 现在怎么分层

可以先按两层来理解：

- 核心调度层
  - `scheduler::Task`
  - `scheduler::Scheduler`
  - `scheduler::Memory`
  - `scheduler::Redis`
  - 这一层只关心 task 如何在 `ready / delayed / inflight` 之间流转
- checkpoint 持久化层
  - `scheduler::checkpoint::{Checkpoint, Counts, Persist, File, Redis, Memory}`
  - 这一层只关心“如何把当前调度状态导出、保存、恢复”

所以：

- `scheduler::Memory` 是纯内存 scheduler
- `scheduler::checkpoint::Memory` 是 `scheduler::Memory + Persist` 的包装
- `scheduler::Redis` 是原生 Redis scheduler
- `scheduler::checkpoint::Redis` 不是 scheduler，它只是 checkpoint 的 Redis 持久化实现
- 如果用户要扩展自己的 scheduler / checkpoint 后端，分别实现 `scheduler::Scheduler` / `scheduler::checkpoint::Persist`

### 用户怎么指定 scheduler

如果默认组件足够，直接用 `Engine::new()`。
如果要自定义 `scheduler`、`http` 或 `browser`，再用 `Engine::from_parts(scheduler, http, browser)`。

- `Engine::new()` 默认就是 `scheduler::Memory + download::Http + download::Browser`
- `Engine::default()` 与 `Engine::new()` 等价
- 如果想保留默认 memory scheduler、但替换下载器，可以用 `Engine::with_downloaders(http, browser)`
- `checkpoint` 只有显式启用时才参与；当前默认内置后端是 `scheduler::checkpoint::File::default()`
- 默认 checkpoint 文件路径是 `output/scheduler-checkpoint.json`
- `scheduler::checkpoint::Memory::default()` 只是“memory scheduler + file checkpoint”的便捷组合
- 如果需要从默认 checkpoint 文件恢复：`scheduler::checkpoint::Memory::load_default().await?`
- 如果需要原生 durable scheduler：直接传 `scheduler::Redis::new(...)`
- 如果需要链式挂 checkpoint：`Engine::new().with_checkpoint(...)`
- 如果需要链式从 checkpoint 恢复：`Engine::new().load_checkpoint(...).await?`

最直接可以这样理解：

```rust
use halo_spider::download::{Browser, Http};
use halo_spider::engine::Engine;
use halo_spider::scheduler;

let memory_engine = Engine::new();

let same_memory_engine = Engine::default();

let custom_downloader_engine = Engine::with_downloaders(Http::default(), Browser::default());

let checkpoint_engine = Engine::new()
    .with_checkpoint(scheduler::checkpoint::File::default());

let redis_engine = Engine::new()
    .with_scheduler(scheduler::Redis::new("redis://127.0.0.1:6379", "kun:scheduler"));
```

### `scheduler::checkpoint::{Checkpoint, Counts, Persist}` 是什么

这三个不是三种新的调度状态，而是“调度状态边界”这一层的三个类型：

- `Checkpoint`
  - 一份完整的 scheduler 状态快照
  - 里面直接带着 `ready / delayed / inflight` 三组任务
  - 适合做导出、恢复、序列化、持久化边界
- `Counts`
  - `Checkpoint` 或 `Memory` 当前状态的轻量计数视图
  - 只关心三组里各有多少任务，不携带具体任务内容
  - 适合做监控、调试、断言
- `Persist`
  - 持久化这份状态快照的抽象接口
  - 对外暴露的能力只有 `load()` 和 `save()`
  - 当前文件与 Redis checkpoint 持久化都实现了这个 trait；后续如果要接别的后端，也继续实现这个 trait

所以这里真正的“状态”仍然只有 `ready / delayed / inflight`。  
`Checkpoint / Counts / Persist` 是围绕这些状态做导出、统计、落盘的类型名。

这里的 `checkpoint` 和 item 最终落地的 `store` 是两条边界：

- `checkpoint` 只负责保存 scheduler 当前的任务流转状态，解决暂停、恢复、crash-safe 这类问题
- `store` 只负责 item 的最终持久化或投递，比如文件、数据库、Webhook、Redis、Kafka

### `Scheduler` trait 的几个动作分别是什么意思

当前 `Scheduler` trait 里的几个动作，语义可以直接按任务流转来理解：

- `enqueue(task)`
  - 把任务放进 scheduler
  - 如果任务已经到执行时间，就进入 `ready`
  - 如果任务还没到时间，就进入 `delayed`
  - 如果多个任务都 ready，当前 `Memory` 会优先按更高 `priority`、再按更低 `depth` 排序
- `take_ready()`
  - 取出一个 `ready` 任务给 engine 执行
  - 同时把它转成 `inflight`
- `complete(task_id)`
  - 表示这个 `inflight` 任务已经成功完成
  - scheduler 会把它从状态里移除
- `requeue(task_id)`
  - 表示这次执行没有完成
  - scheduler 会把这个任务重新放回后续可执行流程
- `has_pending()`
  - 用来判断 scheduler 里是否还存在未完成任务

### 当前边界

- 当前内置了：
  - `scheduler::Memory`
  - `scheduler::Redis`
  - `scheduler::checkpoint::File`
  - `scheduler::checkpoint::Redis`
  - `scheduler::checkpoint::Memory`
- `Task` 现在已经支持最小调度元数据：`priority` 与 `depth`
- `Memory` 已经支持 stable task identity，不再只按 URL 跟踪任务
- `Memory` 支持导出/恢复当前内存 checkpoint
- `Redis` 直接实现了 `Scheduler`，把 ready / delayed / inflight 三组任务状态持久化在 Redis keyspace 里
- `scheduler::checkpoint::Memory` 会在调度状态变化后自动把 checkpoint 保存到共享 `Persist`
- 当前 durable 能力已经提供文件、Redis 两种 checkpoint 持久化，也已经提供直接基于 Redis 的 durable scheduler；这仍不代表已经具备分布式协调或更强事务语义

后续如果补更多 scheduler / checkpoint 后端，也继续是“同一套 trait，不同存储实现”，而不是重写一套新的任务语义。

这部分的设计目标是：先把任务状态和状态边界讲清楚，再去补 durable 实现。

## Pipeline

`pipeline` 是唯一的 item 处理链路。

- spider / callback / DSL step 产出的 item，都走 `pipeline.process()`
- 当前 engine 只保留一个显式 `with_pipeline(...)` 插槽
- pipeline 可以修改 item，也可以通过返回 `Ok(false)` 显式丢弃 item
- 如果 pipeline 返回错误，当前任务显式失败
- 如果 pipeline 保留该 item，引擎会继续把它交给最终 `store`
- 需要跨请求透传上下文时，优先走 `request.meta`，最终 item 一般应在最后一个 parse/callback 里组装，而不是让 pipeline/store 承担隐藏状态拼装

如果你需要多个处理步骤，当前更推荐把顺序逻辑收进一个自定义 pipeline 类型，而不是再暴露 `with_pipeline((A, B))` 这类元组组合语义。

## Store

`store` 是最终持久化或投递边界。

- 通过 pipeline 保留下来的 item，最终都会进入同一个 `store` 边界
- engine 会把同一次 `parse()` / callback 输出里保留下来的 items 收成一批，并优先调用 `store.batch_write()`
- 默认 `Store::batch_write()` 会回退为逐条调用 `store.write()`
- store 可以按自己的底层能力覆盖 `batch_write()`，例如合并文件追加、减少数据库往返、或一次发送多条消息
- `Engine::new()` 默认使用 `store::File::default()`
- 默认 JSON Lines 路径是 `output/<spider_name>.jsonl`
- `Engine::default()` 与 `Engine::new()` 等价

当前内置实现：

- `store::Memory`
- `store::File`
- `store::Sqlite`
- `store::Webhook`
- `store::Redis`
- `store::Kafka`

`store::Memory` 适合测试、断言或内存内调试。

`store::Sqlite` 当前的最小语义是：

- `open()` 时自动建库、建表，但不自动清空旧数据
- 每条 item 都会写入一份完整 `item_json`
- 可以用 `with_field_column(field, column, FieldColumnType::...)` 显式映射字段列
- 缺失字段写 `NULL`
- 显式字段列当前支持 `Text`、`Integer`、`Real`、`Bool`、`Json`
- 映射列不会做隐式类型转换；如果 item 字段值类型不匹配，会返回显式 store error

`store::Webhook` 当前的最小语义是：

- 把完整 item JSON 推送到配置的 HTTP endpoint
- 当前支持 `POST` 与 `PUT`
- 支持追加固定请求头
- 如果目标接口返回非 `2xx`，store 返回显式 error

`store::Redis` 当前的最小语义是：

- 使用 `redis://` URL 连接 Redis
- 当前支持最小 `AUTH` 与 `SELECT` 初始化语义
- `Redis::new(...)` 把完整 item JSON 通过 `SADD` 写入目标 set
- `batch_write()` 会把多个 item JSON 合并到同一个 `SADD key value...` 命令里
- 如果 Redis 返回错误 reply，store 返回显式 error
- 当前不支持 `rediss://`、cluster、Redis protocol pipelining、stream consumer group 或更复杂的 Redis 拓扑能力

`store::Kafka` 当前的最小语义是：

- 使用 `Kafka::new(brokers, topic)` 创建内置 Kafka store
- 每条 item 都以完整 item JSON 作为消息 value 写入目标 topic
- `batch_write()` 会在同一次 store 调用里连续发送多条 item JSON 消息
- 如果 Kafka producer 返回投递错误，store 返回显式 error
- 当前不支持 message key、headers、显式 partition、事务、schema registry 或 consumer/group 这类更高阶 Kafka 语义

如果你要接自己的最终存储后端，也不需要等内置实现；直接实现 `store::Store` 即可。

- `examples/elasticsearch.rs` 展示了完整的自定义 Elasticsearch store
- 这个示例里单条写入走 `_doc`，批量写入走 `_bulk`
- 自定义 store 仍然挂在同一条 `parse -> item -> pipeline -> store` 主链上
- PostgreSQL 这类外部系统现在也建议沿用同样的自定义 `Store` 方式接入

后续仍应继续扩展在 `store` 这一层的输出类型包括：

- 数据库：更多数据库 store 或更完整的数据库写入能力
- 文件：除了当前默认 `File` 之外的其它文件格式或批量落盘 store
- API 推送：更完整的 webhook、HTTP API、第三方服务推送 store
- 消息队列：其它 MQ store，或更完整的 Kafka 能力扩展

这些能力当前还没有内置实现，但最终输出扩展点已经收口到 `store` 这一条线上了。

这部分的设计目标是：item 处理走 `pipeline`，最终输出走 `store`，语义清楚但仍然保持同一条主链。

## Stats

当前 `engine` 已内置最小运行时计数能力。

- 通过 `Engine::stats()` 可以读取一份 `stats::Snapshot`
- 当前快照字段包括：`request_count`、`response_count`、`error_count`、`retry_count`、`item_count`、`pipeline_drop_count`
- 这些计数是单个 engine 实例生命周期内的累计值
- `request_count` 表示实际开始下载的请求次数
- `response_count` 表示成功拿到 `Response` 的次数
- `retry_count` 表示任务被重新入队重试的次数
- `item_count` 表示最终通过 pipeline 并成功写入 store 的 item 数
- `pipeline_drop_count` 表示被 pipeline 显式丢弃的 item 数

当前边界也需要明确：

- 这还是最小内存内计数，不是完整 metrics backend
- 还没有内置 Prometheus、OpenTelemetry 或其它 exporter
- 还没有把 HTTP cache / conditional request 这些 runtime 策略一起纳入统一观测面板；这块当前已明确放到 `P3`

## Robots

当前已经有最小 `robots.txt` 抓取策略。

- 默认关闭；需要显式调用 `Settings::with_robots_obey(true)` 才会启用
- 开启后，引擎会在真正下载前检查当前请求是否被目标站点的 `robots.txt` 允许
- 当前按 `scheme://host[:port]` 做 origin 级内存缓存，同一个 origin 只拉一次 `robots.txt`
- `robots` 使用的 user-agent 优先取 `Settings::with_robots_user_agent(...)`；如果没有显式设置，就回退到当前 `spider.name()`

当前已补的最小规则语义：

- 支持 `User-agent`
- 支持 `Allow`
- 支持 `Disallow`
- 匹配方式是最小前缀匹配
- 多个规则同时命中时，优先使用更长路径；同长度时 `Allow` 优先于 `Disallow`
- 如果没有匹配规则，默认允许抓取

当前边界也要明确：

- 只覆盖 HTTP / HTTPS URL；其它 scheme 当前直接放行
- `404 robots.txt` 当前视为允许全部
- `401` / `403 robots.txt` 当前视为拒绝全部
- 其它抓取失败或非成功状态当前走 fail-open，记录日志后允许继续请求
- 还没有补 `Crawl-delay`、`Sitemap`、更完整 wildcard 语义、持久化 cache 或更复杂的站点级策略

## Parser

当前 parser 能力以“已经稳定可用的解析方式”为主：

- CSS
- JSON
- XML
- Regex
- Feed
- AI selector

当前仍未收敛的能力：

- HTML XPath
- OCR
- 更完整的 parse 后处理能力

所以现在如果是 HTML 页面，优先建议用 CSS 选择器，不建议把 XPath 当成已经可靠的 HTML 能力。

当前已经落下的最小 query transform 能力：

- `fallback(...)`：查询结果为空时用另一个 query 兜底
- `fallback_many([...])`：按顺序尝试多个 query，返回第一个非空结果
- `field("key")`：从结构化结果里提取对象字段
- `index(i)`：从数组结果里提取指定位置元素
- `flatten()`：把顶层数组结果展开成普通 value 列表，方便继续链式处理
- `skip(n)` / `take(n)` / `last()`：对结果列表做最小切片，方便拿分页、尾项或固定窗口
- `join(...)`：把多段提取结果串成一个文本值
- `compact()`：丢掉 `null` 和空字符串结果，保留有效值
- `trim()`：显式裁掉当前结果里的字符串首尾空白；会递归处理数组和对象里的字符串值
- `first_non_empty()`：跳过空值，保留第一个有效结果
- `dedup()`：按原始顺序去重，保留第一次出现的值
- `split(delimiter)`：把字符串值按分隔符拆成多个结果；适合 meta keywords、标签串、逗号分隔字段
- `replace(from, to)`：对字符串值做最小文本替换
- `normalize_whitespace()`：统一折叠空白字符
- `resolve_url(base_url)`：把相对 URL 按给定 base URL 解析成绝对 URL；当前只处理顶层字符串值，遇到空串或非字符串会显式返回 parse error
- `parse_number()`：把字符串或数字值收口成数值，失败时显式返回 parse error
- `parse_bool()`：把 `true/false/1/0` 这类最小布尔文本收口成布尔值，失败时显式返回 parse error
- `parse_json()`：把嵌入页面或脚本里的 JSON 文本解析成结构化值，方便继续 `.field(...)`、`.index(...)` 这类链式读取
- `parse_datetime()`：把常见时间文本收口成规范化时间字符串；当前优先支持 RFC 3339 / Jiff temporal 文本，并补了最小常见 civil 格式兜底，例如 `2026-04-01 08:30`、`2026/04/01`
- `parse_datetime_with_format(format)`：按显式 `strptime` 格式解析时间文本，适合抓取中常见的非标准日期时间布局；解析后同样输出规范化时间字符串

当前也补了最小 query 级约束：

- `require_non_empty()`：要求至少有一个非空结果，否则直接返回 parse error
- `require_one()`：要求恰好只有一个非空结果，否则直接返回 parse error

更丰富的 map/filter 规则、跨 query 组合策略与结构化后处理 still 没有完全统一收口；当前已经覆盖多 query 兜底、结构投影、数组拉平、结果切片/去重、string transform、URL resolve、embedded JSON parse、number/bool/datetime conversion 与 query-level assertions。

## Validation

`validate` 是底层共享能力，不应该只存在于 DSL 配置里。

- 当前已经有共享 `Validation` / `ValidationRule` 结构
- DSL 可以编译到这套共享 validation 定义
- 代码爬虫现在已经可以直接调用 `validator::validate_fields()` / `validator::validate_item()`，也可以用 `validator::validate_fields_report()` / `validator::validate_item_report()` 收集多条错误
- validation 是显式启用的：只有传入的 `Validation` 会执行；没有配置规则的字段不会被默认校验，字段缺失时也只有 `required` 才报错，其它规则会直接跳过
- `Validation.field` 现在已经支持最小字段路径：`meta.title`、`authors[0].name`、`tags[]`、`articles[].title`
- 如果使用数组展开路径，当前语义是“对展开后的每个值逐个校验”；报错时会尽量返回具体路径，例如 `articles[1].title`
- 当前还补了更直白的类型化约束入口：
  - 文本：`with_min_length(...)`、`with_max_length(...)`
  - 列表：`with_min_items(...)`、`with_max_items(...)`
  - 对象：`with_min_fields(...)`、`with_max_fields(...)`、`with_required_fields([...])`
- 当前也支持 `ValidationTransform` 链式转换后再校验：
  - 文本规范化：`Trim`、`NormalizeWhitespace`
  - 标量转换：`ParseNumber`、`ParseBool`、`ParseDatetime`
  - 典型用法是先把抓取出来的字符串值收口，再按最终 `ValidationType` 与规则继续校验
- 当前也支持嵌套子规则：
  - `with_object_validations([...])`：对对象值内部字段继续做相对路径校验
  - `with_each_validations([...])`：对列表中的每个成员继续做相对路径或根值校验
  - 子规则报错时会保留完整前缀路径，例如 `articles[1].title`
- 当前也支持组合约束：
  - `with_all_of([...])`：当前作用域下的多条验证都必须通过
  - `with_any_of([...])`：当前作用域下至少一条验证通过
  - `with_one_of([...])`：当前作用域下恰好一条验证通过
  - `with_mutually_exclusive([...])`：当前作用域下至多一条验证通过
  - 顶层作用域可以用 `Validation::root()` 显式声明
  - 可选字段在组合约束里会被当成“skipped”而不是“passed”，这样 `any_of / one_of` 不会因为字段没出现而误判通过
- 当前也支持条件约束：
  - `with_when_exists(...)`、`with_when_missing(...)`
  - `with_when_equals(...)`、`with_when_not_equals(...)`
  - `with_required_when_exists(...)`、`with_required_when_missing(...)`
  - `with_required_when_equals(...)`、`with_required_when_not_equals(...)`
  - 多个条件当前按 `AND` 语义组合；条件路径也走同一套字段路径解析，并且在嵌套对象/列表作用域里按相对路径解析
  - 典型场景是 “`type == video` 时 `duration` 必填” 或 “某个伴随字段缺失时才要求另一字段出现”

这块后续还会继续补更高阶的运行时失败策略映射，以及更复杂的跨字段派生条件。

这部分的设计目标是：先把 validation 做成代码可直接调用的底层能力，再映射到 DSL。

## Plugins

当前 plugin 体系只把 `middleware` 当成已经落地的运行时能力。

- `middleware`：已支持
- `rules` / `provider` / `storage`：当前只保留命名空间，不作为已落地能力承诺

这样做是为了避免注册表看起来什么都能扩展，但 engine 实际只接了一部分。

## DSL 当前定位

当前阶段，DSL 先后置，不继续扩配置面。

它的定位已经明确：

- 不是另一套运行时
- 不是重新发明一套调度、重试、去重、输出机制
- 而是把代码爬虫已有的底层能力配置化

也就是说，正确方向应该是：

`代码能力先做实` -> `抽成稳定底层接口` -> `DSL 再映射这些接口`

而不是反过来让 DSL 字段去主导底层模型。
