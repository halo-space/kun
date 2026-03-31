# 功能说明

这份文档按功能模块整理当前 `kun` 已实现的底层能力、运行边界和主要命名含义。
README 负责总览，这里负责把每个模块现在到底能做什么、还缺什么讲清楚。

## 阅读方式

如果你是第一次看这个项目，推荐按这个顺序理解：

- 先看 `Request` / `Download` / `Response`，理解“请求如何发出、结果如何返回”
- 再看 `Scheduler` / `Pipeline`，理解“任务如何流转、item 如何处理”
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
- 已支持内置 `fingerprint_profile = desktop_zh_cn | desktop_en_us`
- 已支持最小 `stealth = true` bootstrap，覆盖 `navigator.webdriver`、`navigator.languages`、`navigator.platform`、最小 `window.chrome` 与 notifications permissions 查询补丁
- 同一个 browser session 会复用稳定的 user data dir，并做最小串行化，避免并发抢 profile

当前仍未收敛、并会继续显式报错或保留空白的部分：

- 自定义 `fingerprint_profile` 名称
- 更完整的第三方 stealth 套件或更高阶浏览器指纹伪装能力
- `ip_address`、`certificate` 这类 Playwright 当前接口拿不到的响应侧字段

这部分的设计目标是：HTTP 和 browser 只是两种下载方式，最终都回到统一请求语义。

## Response

`Response` 是下载结果的统一表示。

- `body` 是原始响应字节
- `text` 是从 `body` 解码出来的字符串视图
- 当前文本解码优先顺序是：BOM -> `Content-Type charset` -> 文档内编码声明 -> UTF-8 lossy
- HTTP 和 browser 最终都要回到统一的 `Response` 语义上，方便 parser、callback、pipeline 复用

## Scheduler

`scheduler` 负责管理“已发现但尚未完成”的任务流转。

当前内置实现是 `scheduler::Memory`，它把任务分成三组状态：

- `ready`：已经可以立即执行的任务
- `delayed`：因为 delay / retry backoff 等原因，还没到执行时间的任务
- `inflight`：已经取出执行、正在运行、尚未完成或重新入队的任务

当前对外更推荐直接使用 `scheduler` 根导出的类型：

- `scheduler::Memory`
- `scheduler::Task`
- `scheduler::TaskId`
- `scheduler::Scheduler`

### `scheduler::state::{Snapshot, Counts, Store}` 是什么

这三个不是三种新的调度状态，而是“调度状态边界”这一层的三个类型：

- `Snapshot`
  - 一份完整的 scheduler 状态快照
  - 里面直接带着 `ready / delayed / inflight` 三组任务
  - 适合做导出、恢复、序列化、持久化边界
- `Counts`
  - `Snapshot` 或 `Memory` 当前状态的轻量计数视图
  - 只关心三组里各有多少任务，不携带具体任务内容
  - 适合做监控、调试、断言
- `Store`
  - 持久化这份状态快照的抽象接口
  - 对外暴露的能力只有 `load()` 和 `save()`
  - 后续如果做 SQLite、Redis、磁盘文件这类 durable scheduler state，就实现这个 trait

所以这里真正的“状态”仍然只有 `ready / delayed / inflight`。  
`Snapshot / Counts / Store` 是围绕这些状态做导出、统计、落盘的类型名。

### `Scheduler` trait 的几个动作分别是什么意思

当前 `Scheduler` trait 里的几个动作，语义可以直接按任务流转来理解：

- `enqueue(task)`
  - 把任务放进 scheduler
  - 如果任务已经到执行时间，就进入 `ready`
  - 如果任务还没到时间，就进入 `delayed`
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

- 当前内置的只有 `scheduler::Memory`
- 它已经支持 stable task identity，不再只按 URL 跟踪任务
- 它支持导出/恢复当前内存状态
- 它还不是 crash-safe durable scheduler

后续如果补数据库版 scheduler，会是“同一套 trait，不同存储实现”，而不是重写一套新的任务语义。

这部分的设计目标是：先把任务状态和状态边界讲清楚，再去补 durable 实现。

## Pipeline

`pipeline` 是唯一的 item 处理链路。

- spider / callback / DSL step 产出的 item，都走 `pipeline.process()`
- `with_pipeline((A, B))` 表示固定串行链路 `A -> B`
- 如果前一段返回 `Ok(false)`，后一段不会继续执行
- 如果 pipeline 返回错误，当前任务显式失败

当前内置实现：

- `pipeline::Memory`
- `pipeline::JsonLines`

数据库、消息队列、对象存储等输出还没有内置实现，但扩展点已经收口到 pipeline 这一条线上了。

这部分的设计目标是：所有 item 输出都走一条链，不再分叉出另一套 sink 语义。

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
- parse 后处理能力

所以现在如果是 HTML 页面，优先建议用 CSS 选择器，不建议把 XPath 当成已经可靠的 HTML 能力。

## Validation

`validate` 是底层共享能力，不应该只存在于 DSL 配置里。

- 当前已经有共享 validation plan
- DSL 可以编译到这套 plan
- 代码爬虫现在已经可以直接调用 `validator::validate_fields()` / `validator::validate_item()`

这块后续还会继续补更完整的规则集和失败策略。

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
