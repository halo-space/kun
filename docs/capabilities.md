# 功能说明

这份文档按功能模块整理当前 `kun` 已实现的底层能力、运行边界和主要命名含义。
README 负责总览，这里负责把每个模块现在到底能做什么、还缺什么讲清楚。

## 阅读方式

如果你是第一次看这个项目，推荐按这个顺序理解：

- 先看 `Request` / `Download` / `Response`，理解“请求如何发出、结果如何返回”
- 再看 `Scheduler` / `Pipeline` / `Store`，理解“任务如何流转、item 如何处理、最终如何落地”
- 最后看 `Validation` / `Plugins` / `DSL 当前定位`，理解扩展边界和后续方向

## 与 Scrapy 的主要差距

如果只看“代码爬虫底层能力”这一层，而不看 DSL，当前和 Scrapy 更完整运行时相比，最主要的剩余缺口是：

- 更完整的观测能力：exporter、trace 链路、跨 job 运维视角还没补完
- 更强的分布式 scheduler 协调与事务语义还没完全统一
- browser 已支持结构化 custom profile 与显式 `session_reuse`；当前剩余缺口主要是更高阶 stealth 套件和跨 engine 更完整的指纹伪装
- plugin 自动装载与 DSL 对齐仍明显落后于底层 runtime 能力

## Request

`Request` 是统一的执行单元。

- 代码爬虫里手写的请求，和后续 DSL 生成的请求，本质上都应该落成同一个 `Request`
- 如果 spider 需要从入口就带上 cookies、proxy、session、browser mode 这类能力，可以直接覆写 `build_start_requests()` 返回完整 `Request`；默认实现仍然只是把 `build_start_urls()` 包成 `Request::new(...)`
- rules / DSL 这条线现在也不会再单独发明请求结构：start request 与 step follow request 都会先走共享 `Request` / `response.follow()` 语义，再把目标 step 的 `fetch.request` / `fetch.browser` 覆盖应用上去
- 当前已经接线的共享请求语义包括：`method`、`headers`、`body`、`timeout`、`proxy`、`session`、request cookies
- `meta` 是请求级上下文参数，用来挂当前请求和后续链路要透传的数据；它更接近 Scrapy 的函数参数/上下文，而不是框架私自塞内部控制字段的地方
- `kwargs` 是显式给 callback / errback 使用的回调上下文参数；它和 `meta` 分开建模，不拿来承载框架内部控制字段
- 普通 callback 可以通过 `response.kwarg("name")` 读取 `kwargs`；errback 可以通过 `failure.kwarg("name")` 读取同一份显式上下文
- `errback` 现在也已经是 `Request` 的一等能力；下载失败或 spider callback 失败时，引擎会把失败上下文分发到对应 errback
- `follow()` 会继承请求级共享语义，再按子请求显式覆盖

这部分的设计目标是：不管请求最后走 HTTP 还是 browser，入口模型都尽量一致。

## Download

当前下载能力分成两类实现，但它们都只是 `Request` 的不同执行方式，不是两套互相割裂的运行时。

### `download::Http`

- 基于 `reqwest`
- 已接线真实的 timeout、proxy、redirect、cookie jar、session cookies
- 已支持最小 `HTTP cache / conditional request`：开启 `Settings::with_http_cache(true)` 后，会基于缓存的 `ETag / Last-Modified` 自动补条件请求头，并在 `304 Not Modified` 时回填缓存 body
- `Response.body` 保存原始字节
- `Response.text` 从 `body` 按统一解码规则派生

### `download::Browser`

- 当前走 `playwright-rs`
- 用于打开页面、执行浏览器导航、拿渲染后的 HTML
- 已支持最小的 `method`、`body`、`headers`、`timeout`、`proxy`、request cookies、session
- 已支持 `wait_for` 这类页面就绪等待配置，用于在取 HTML 前等待目标内容出现
- 已支持内置 `fingerprint_profile = desktop_zh_cn | desktop_en_us | desktop_en_gb | desktop_ja_jp | desktop_de_de | desktop_fr_fr`
- 已支持结构化 `custom_fingerprint_profile`
- 已支持显式 `session_reuse = storage | context | page`
- 已支持更完整但仍然克制的 `stealth = true` bootstrap，覆盖 `navigator.webdriver`、`navigator.language(s)`、`navigator.platform`、`navigator.vendor`、`hardwareConcurrency`、`deviceMemory`、`maxTouchPoints`、`plugins`、`mimeTypes`、`pdfViewerEnabled`、screen depth、notifications permissions 查询补丁，以及 Chromium 路线的最小 `window.chrome` / `navigator.userAgentData`
- 同一个 browser session 会复用稳定的 user data dir，并做最小串行化；如果显式启用 `session_reuse`，还可以进一步复用 live context 或 live page
- user data dir、临时 profile 目录与会话锁这条路径已经改成 async runtime 更友好的实现，不再依赖明显的同步文件 I/O 热路径

内置 `fingerprint_profile` 的稳定映射：

| profile | user_agent family | locale | timezone | languages | platform |
| --- | --- | --- | --- | --- | --- |
| `desktop_zh_cn` | Chrome 136 / Windows 10 x64 | `zh-CN` | `Asia/Shanghai` | `["zh-CN", "zh", "en"]` | `Win32` |
| `desktop_en_us` | Chrome 136 / Windows 10 x64 | `en-US` | `America/New_York` | `["en-US", "en"]` | `Win32` |
| `desktop_en_gb` | Chrome 136 / Windows 10 x64 | `en-GB` | `Europe/London` | `["en-GB", "en"]` | `Win32` |
| `desktop_ja_jp` | Chrome 136 / Windows 10 x64 | `ja-JP` | `Asia/Tokyo` | `["ja-JP", "ja", "en-US", "en"]` | `Win32` |
| `desktop_de_de` | Chrome 136 / Windows 10 x64 | `de-DE` | `Europe/Berlin` | `["de-DE", "de", "en"]` | `Win32` |
| `desktop_fr_fr` | Chrome 136 / Windows 10 x64 | `fr-FR` | `Europe/Paris` | `["fr-FR", "fr", "en"]` | `Win32` |

当前仍未收敛、并会继续显式报错或保留空白的部分：

- 自定义 `fingerprint_profile` 名称注册机制
- 更完整的第三方 stealth 套件或更高阶浏览器指纹伪装能力
- `ip_address`、`certificate` 这类 Playwright 当前接口拿不到的响应侧字段

这里刻意保持一个边界：

- browser 仍然只是渲染型下载器，不扩成通用自动化框架
- fingerprint profile 目前只提供稳定内置 preset，不承诺“跨所有 engine 的品牌级完美伪装”

这部分的设计目标是：HTTP 和 browser 只是两种下载方式，最终都回到统一请求语义。
browser 在这里的角色是“渲染型下载器”，不是另起一套通用浏览器自动化 runtime；当前只保留导航、等待页面就绪和返回最终 HTML 这类爬虫抓取直接需要的能力。

## Runtime 调速

当前已经有两类下载调速能力：

- 固定调速：`download_delay` 会继续编译成固定 `interval_gate`
- 自适应调速：`Settings::with_auto_throttle(true)` 会改成 `auto_throttle` 中间件，按 origin 维护动态 delay

最小 `AutoThrottle` 的当前语义：

- `download_delay` 在开启 `auto_throttle` 后表示初始/最小 delay
- `with_auto_throttle_target_concurrency(...)` 表示每个 origin 的目标并发
- `with_auto_throttle_max_delay(...)` 表示 delay 上限
- 成功响应会按最近延迟逐步调整后续 delay
- 下载异常以及 `429 / 5xx` 响应会把后续 delay 抬高
- 如果同一个 origin 的 inflight 请求已经达到 `target_concurrency`，后续请求会主动退避

这层仍然只是现有下载链路上的 middleware 组合，不是单独再造一套 runtime。

## HTTP Cache

当前已经有一版最小 `HTTP cache / conditional request` 能力。

- 通过 `Settings::with_http_cache(true)` 开启
- 当前实现形态是 `http_cache` download middleware
- 只作用于 HTTP `GET` 请求
- 当前 key 语义是规范化后的完整 URL，包含 `request.http.query`
- 默认 backend 是 `middleware::http_cache::Memory`
- 当前也已提供内置 `middleware::http_cache::File`，用于把缓存条目持久化到磁盘 JSON 文件；`File::default()` 的路径是 `output/http-cache.json`
- 当前支持 `ttl`；默认按 `24h` 复用缓存条目，可以通过 `Settings::with_http_cache_ttl(...)` 覆盖，或通过 `without_http_cache_ttl()` 关闭自动过期
- 当前支持两种策略：
  `Strategy::Validators` 只缓存 `ETag / Last-Modified`
  `Strategy::Response` 会连同响应 body 一起缓存，并在服务端返回 `304 Not Modified` 时回填成正常 `Response`
- 回填后的 `Response.flags` 会追加 `http_cache`

当前边界也需要明确：

- 当前不做 `Cache-Control` / `Expires` / `Vary` 这类更完整的 HTTP 缓存语义
- 当前 `Engine::stats()` 已补 `http_cache_hit_count`、`http_cache_revalidate_count`、`http_cache_store_count` 与 `http_cache_miss_count`

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
- `scheduler::Redis` 是原生 Redis scheduler，并且现在带最小 `lease_timeout` stale inflight reclaim
- `scheduler::checkpoint::Redis` 不是 scheduler，它只是 checkpoint 的 Redis 持久化实现
- 如果用户要扩展自己的 scheduler / checkpoint 后端，分别实现 `scheduler::Scheduler` / `scheduler::checkpoint::Persist`

### 用户怎么指定 scheduler

如果默认组件足够，直接用 `Engine::new()`。
如果想保留默认 scheduler、只替换下载器，优先用 `.with_http(...)` / `.with_browser(...)`。
如果要连 `scheduler` 一起换掉，再用 `Engine::from_parts(scheduler, http, browser)`。
如果要替换默认去重实现，再继续链 `.with_dedup(...)`。
如果要替换默认 robots policy，再继续链 `.with_robots(...)`。

- `Engine::new()` 默认就是 `scheduler::Memory + download::Http + download::Browser + dedup::Memory + robots::Memory`
- `Engine::default()` 与 `Engine::new()` 等价
- 如果想只替换 HTTP 下载器，可以用 `.with_http(...)`
- 如果想只替换 browser 下载器，可以用 `.with_browser(...)`
- 如果想同时替换两个下载器，可以链 `.with_http(...).with_browser(...)`
- `Engine::with_downloaders(http, browser)` 继续保留，作为默认 memory scheduler 下的一次性快捷写法
- 当前不再单独抽一个 `queue` 组件；任务排队与状态流转统一就是 `scheduler::Scheduler` 这条边界
- 如果想关闭默认去重，可以显式用 `.with_dedup(dedup::Noop)`
- 如果想用有界内存的近似去重，可以显式用 `.with_dedup(dedup::Bloom::default())`
- 如果想自定义请求指纹规则或底层存储，也可以实现 `dedup::Dedup` 再挂到 `.with_dedup(...)`
- 如果是手动往引擎里塞 request，优先用 `engine.enqueue(request).await?`；直接调 `engine.scheduler.enqueue(...)` 属于低层入口，会绕过 dedup 组件
- `robots` 是否启用和使用哪个 user-agent 仍由 `Settings::with_robots_obey(...)` / `Settings::with_robots_user_agent(...)` 控制；如果要替换默认 robots policy 实现，用 `.with_robots(...)`
- `checkpoint` 只有显式启用时才参与；当前默认内置后端是 `scheduler::checkpoint::File::default()`
- 默认 checkpoint 文件路径是 `output/scheduler-checkpoint.json`
- `scheduler::checkpoint::Memory::default()` 只是“memory scheduler + file checkpoint”的便捷组合
- 如果需要从默认 checkpoint 文件恢复：`scheduler::checkpoint::Memory::load_default().await?`
- 如果需要原生 durable scheduler：直接传 `scheduler::Redis::new(...)`
- `scheduler::Redis` 默认会给 `inflight` task 建一个最小 lease；worker 崩溃或长时间不处理时，后续访问同 namespace 会把 stale `inflight` task 回收到 `ready / delayed`
- `scheduler::Redis` 现在会通过 Redis 脚本原子完成 `enqueue / claim / complete / requeue / reclaim` 这类关键状态迁移；多个 worker 共享同一个 namespace 时，不会再因为“先读 ready 再分步迁移”而重复领取同一条 task
- `scheduler::Redis` 现在还显式支持 `worker_id` ownership 校验，以及 engine 运行时的 heartbeat 续租
- `scheduler::Redis::snapshot().await?` 可以直接读取单个 namespace 当前这一刻的运行时快照
- `snapshot.inflight_tasks` 会直接带出每条 inflight task 的 task id、url、worker、lease、deadline 与 priority/depth 元信息
- 如果同一个 Redis 里有多个 job / namespace，可以用 `scheduler::Redis::namespaces_with_prefix(...)` 和 `scheduler::Redis::namespace_snapshots_with_prefix(...)` 做跨 job 运维读取
- 如果需要调整恢复窗口：`scheduler::Redis::new(...).with_lease_timeout(...)`
- 如果需要显式指定 worker 或 heartbeat：`scheduler::Redis::new(...).with_worker_id(...).with_heartbeat_interval(...)`
- 如果明确不想启用这层自动回收：`scheduler::Redis::new(...).without_lease_timeout()`
- 如果需要链式挂 checkpoint：`Engine::new().with_checkpoint(...)`
- 如果需要链式从 checkpoint 恢复：`Engine::new().load_checkpoint(...).await?`
- 更完整的分布式用法说明见 `docs/distributed_scheduler.md`

最直接可以这样理解：

```rust
use halo_spider::dedup;
use halo_spider::download::{Browser, Http};
use halo_spider::engine::Engine;
use halo_spider::robots;
use halo_spider::scheduler;
use halo_spider::settings::Settings;

let memory_engine = Engine::new();

let same_memory_engine = Engine::default();

let custom_downloader_engine = Engine::with_downloaders(Http::default(), Browser::default());

let chained_downloader_engine = Engine::new()
    .with_http(Http::default())
    .with_browser(Browser::default());

let custom_dedup_engine = Engine::new().with_dedup(
    dedup::Memory::new().with_keys([dedup::Key::Url, dedup::Key::Method]),
);

let bloom_dedup_engine = Engine::new().with_dedup(
    dedup::Bloom::new()
        .with_expected_items(500_000)
        .with_false_positive_rate(0.01),
);

let custom_robots_engine = Engine::new()
    .with_robots(robots::Noop)
    .with_settings(Settings::default().with_robots_obey(true));

let checkpoint_engine = Engine::new()
    .with_checkpoint(scheduler::checkpoint::File::default());

let redis_engine = Engine::new()
    .with_scheduler(
        scheduler::Redis::new("redis://127.0.0.1:6379", "kun:scheduler")
            .with_worker_id("news-worker-a")
            .with_lease_timeout(jiff::SignedDuration::from_secs(30))
            .with_heartbeat_interval(jiff::SignedDuration::from_secs(10)),
    );
```

### `dedup::{Dedup, Memory, Bloom, Noop}` 是什么

- `dedup::Dedup`
  - request 去重的统一组件边界
  - 引擎会在 request 进入 scheduler 前调用它
  - 如果用户要自定义去重算法或存储后端，实现这个 trait 即可
- `dedup::Memory`
  - 内置的精确内存去重实现
  - 默认按 URL 去重，也可以通过 `dedup::Key` 组合 method、body、meta 字段做更细粒度指纹
- `dedup::Bloom`
  - 内置的布隆过滤器去重实现
  - 默认也按 URL 指纹去重，但它是近似去重，会有误判边界
  - 当前默认参数是 `expected_items = 100_000`、`false_positive_rate = 0.01`
  - 更适合“请求量很大、愿意接受少量误判来换内存上界”的场景
- `dedup::Noop`
  - 永远放行 request
  - 适合完全关闭框架级 dedup 的场景

当前关于默认策略的明确决策是：

- `Engine::new()` 继续默认使用精确 `dedup::Memory`
- `dedup::Bloom` 作为显式可选组件提供，不默认替换
- 原因是默认行为优先保 correctness，不默认引入布隆误判导致的潜在漏抓

最小自定义 dedup 可以这样写：

```rust
use halo_spider::dedup::Dedup;
use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::request::Request;
use std::collections::HashSet;

struct MethodUrlDedup {
    seen: HashSet<String>,
}

impl Dedup for MethodUrlDedup {
    async fn check_and_insert(&mut self, request: &Request) -> Result<bool, SpiderError> {
        Ok(self
            .seen
            .insert(format!("{}|{}", request.method, request.url)))
    }
}

let engine = Engine::new().with_dedup(MethodUrlDedup {
    seen: HashSet::new(),
});
```

同一份 `dedup::Dedup` trait 也是后续持久化 dedup、远程 dedup 或其它自定义算法的稳定扩展边界；内置 `Memory / Bloom / Noop` 都只是这条边界上的实现。

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

- `checkpoint` 只负责保存 scheduler 当前的任务流转状态，解决暂停、恢复、导出快照这类问题
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
- `Redis` 直接实现了 `Scheduler`，把 ready / delayed / inflight 三组任务状态持久化在 Redis keyspace 里，并会按 `lease_timeout` 回收 stale inflight task
- `Redis` 对 `enqueue / claim / complete / requeue / reclaim / heartbeat` 这些关键迁移已经收口成原子脚本，所以同一个 namespace 上的多 worker 不会再重复 claim 同一条 ready task
- `Redis` 现在还显式校验 `worker_id + lease_id` ownership；旧 lease 或错误 worker 不能再覆盖当前 inflight owner
- `Redis::snapshot()` 现在除了 namespace 级计数，也会带出 `snapshot.workers`，直接给出每个 worker 的 `last_seen / is_stale / inflight_task_ids / next_deadline / lease_timeout / heartbeat_interval`
- `snapshot()`、`counts()`、`checkpoint()` 这类只读入口不会把调用方登记成活跃 worker；真正刷新 worker runtime 的只有 `enqueue / take_ready / complete / requeue / heartbeat`
- `scheduler::checkpoint::Memory` 会在调度状态变化后自动把 checkpoint 保存到共享 `Persist`
- `checkpoint` 恢复的仍然只是保存时那份静态 `ready / delayed / inflight` 快照，不承担 runtime reclaim
- 当前 durable scheduler 的最小运行时语义已经完成：除了文件、Redis 两种 checkpoint 持久化，也已经提供直接基于 Redis 的 durable scheduler；当前这层已经覆盖最小 worker ownership、heartbeat、stale reclaim、namespace snapshot、worker runtime snapshot 与跨 namespace 运维读取入口。

后续如果补更多 scheduler / checkpoint 后端，也继续是“同一套 trait，不同存储实现”，而不是重写一套新的任务语义。

这部分的设计目标是：先把任务状态和状态边界讲清楚，再去补 durable 实现。

如果你把它和常见爬虫框架里的 “queue/frontier” 概念对照着看，当前 kun 这里不再额外拆一个新组件名：

- `scheduler::Scheduler`
  - 就是任务队列与任务状态机的统一边界
  - 负责 enqueue、取 ready task、complete、requeue，以及是否还有 pending task
- `checkpoint`
  - 只是 scheduler 状态的保存/恢复边界
  - 不是新的队列实现

这样做是为了减少名词层级，避免 `queue / frontier / scheduler` 三套名字同时存在造成歧义。

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

`store::File` 当前的最小增强语义是：

- 默认仍然写紧凑 JSON Lines
- 支持 `FileFormat::PrettyJsonBlocks`
- 支持 `with_rotate_items(...)` 与 `with_rotate_bytes(...)`
- rotate 后的文件按编号命名，例如 `items-0001.jsonl`、`items-0002.jsonl`

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
- 支持 `with_retry_limit(...)` 与 `with_retry_backoff(...)`
- 只对请求错误和 `429 / 5xx` 做重试
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
- 支持固定或按 item 字段生成的 message key
- 支持固定或按 item 字段生成的 headers
- `batch_write()` 会在同一次 store 调用里连续发送多条 item JSON 消息
- 如果 Kafka producer 返回投递错误，store 返回显式 error
- 当前仍不支持显式 partition、事务、schema registry 或 consumer/group 这类更高阶 Kafka 语义

如果你要接自己的最终存储后端，也不需要等内置实现；直接实现 `store::Store` 即可。

- `examples/elasticsearch.rs` 展示了完整的自定义 Elasticsearch store
- 这个示例里单条写入走 `_doc`，批量写入走 `_bulk`
- 自定义 store 仍然挂在同一条 `parse -> item -> pipeline -> store` 主链上
- PostgreSQL 这类外部系统现在也建议沿用同样的自定义 `Store` 方式接入

当前内置维护范围是：

- `Memory / File / Sqlite / Webhook / Redis / Kafka`
- 更专门的数据库、对象存储、第三方 API 和复杂 MQ 语义，继续建议通过自定义 `Store` 扩展

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
- 当前快照字段包括：
  `request_count`、`response_count`、`error_count`、`retry_count`、`item_count`、`pipeline_drop_count`
  `dedup_reject_count`、`robots_disallow_count`、`robots_delay_count`
  `http_cache_hit_count`、`http_cache_revalidate_count`、`http_cache_store_count`、`http_cache_miss_count`
  `store_error_count`
- 这些计数是单个 engine 实例生命周期内的累计值
- `scheduler::Redis::snapshot()` 和 `scheduler::Redis::namespace_snapshots_with_prefix(...)` 读的是 durable scheduler namespace 的即时状态，不是这些累计计数
- `request_count` 表示实际开始下载的请求次数
- `response_count` 表示成功拿到 `Response` 的次数
- `retry_count` 表示任务被重新入队重试的次数
- `item_count` 表示最终通过 pipeline 并成功写入 store 的 item 数
- `pipeline_drop_count` 表示被 pipeline 显式丢弃的 item 数
- `dedup_reject_count` 表示请求在进入 scheduler 前被 dedup 拒绝的次数
- `robots_disallow_count` 表示请求在下载前被 robots 直接拒绝的次数
- `robots_delay_count` 表示请求因为 robots `Crawl-delay` 被退避重试的次数
- `http_cache_hit_count` 表示服务端返回 `304` 后成功回填缓存 body 的次数
- `http_cache_revalidate_count` 表示请求因为已有 validator 而带条件请求头回源的次数
- `http_cache_store_count` 表示引擎把可缓存响应写入 http cache backend 的次数
- `http_cache_miss_count` 表示可缓存请求在进入下载前没有命中可复用缓存条目的次数
- `store_error_count` 表示最终写入 store 失败的次数
- 如果需要流式观测，也可以通过 `Engine::with_stats_reporter(...)` 注册最小 reporter 钩子

当前边界也需要明确：

- 这还是最小内存内计数，不是完整 metrics backend
- 还没有内置 Prometheus、OpenTelemetry 或其它 exporter
- `Engine::stats()` 仍然是主读取 API；`with_stats_reporter(...)` 只是为后续 exporter 预留的最小接线点
- 如果需要 durable scheduler 的运行时观测，优先读 `scheduler::Redis::snapshot()` 或 `scheduler::Redis::namespace_snapshots_with_prefix(...)`；如果需要单个 engine 生命周期累计计数，再读 `Engine::stats()`

## Signals / Extensions

当前 `engine` 已提供最小 `signals / extensions` 边界。

- 如果你想拿到最原始的 runtime 事件流，用 `Engine::with_signal_listener(...)`
- 如果你只关心部分 signal kind，可以用 `Engine::with_signal_listener_for([...], ...)`
- 如果你想挂更语义化的扩展，用 `Engine::with_extension(...)` 或 `Engine::with_extension_for([...], ...)`
- `with_extension(...)` 底层复用同一条 signal bus，不会额外引入另一套 runtime
- 当前内置信号类型包括：`spider_opened`、`spider_closed`、`request_scheduled`、`response_received`、`item_scraped`、`spider_error`
- `spider_closed` 会携带最终 `stats::Snapshot`
- `spider_error` 会携带 `request`、可选 `response` 与显式 `SpiderError`
- 当前内置扩展示例是 `extensions::Summary`，会在 `spider_closed` 时输出一份最终统计摘要

最小用法：

```rust
use halo_spider::engine::Engine;
use halo_spider::extensions;

let engine = Engine::new().with_extension(extensions::Summary);
```

如果你只想监听部分 signal kind，可以这样写：

```rust
use halo_spider::engine::Engine;
use halo_spider::signals;

let engine = Engine::new().with_signal_listener_for(
    [signals::Kind::SpiderClosed, signals::Kind::SpiderError],
    my_listener,
);
```

如果你要自定义：

- 实现 `signals::Listener`，再挂到 `.with_signal_listener(...)` 或 `.with_signal_listener_for([...], ...)`
- 或实现 `extensions::Extension`，再挂到 `.with_extension(...)` 或 `.with_extension_for([...], ...)`

当前边界也要明确：

- 这是一条 engine 内部 runtime hook，不是新的 plugin registry
- 当前也还没有持久化事件总线或跨进程分发
- plugin 自动装载能力仍然只落在 `middleware` kind

## Robots

当前已经有一版更完整的 `robots.txt` 抓取策略。

- 默认关闭；需要显式调用 `Settings::with_robots_obey(true)` 才会启用
- 开启后，引擎会在真正下载前检查当前请求是否被目标站点的 `robots.txt` 允许
- 如果命中 `Crawl-delay` 或 `Request-rate`，引擎不会把请求当成永久拒绝，而是按 delay 退避后再重试
- `Request-rate` 当前按 `window / requests` 的均匀间隔最小 delay 解释；如果同时声明 `Crawl-delay` 和 `Request-rate`，当前取更严格的那个 delay
- 当前按 `scheme://host[:port]` 做 origin 级缓存；默认会在 `24h` 的 `cache_ttl` 内直接复用，超出后尝试刷新
- `robots` 使用的 user-agent 优先取 `Settings::with_robots_user_agent(...)`；如果没有显式设置，就回退到当前 `spider.name()`
- 默认 robots 组件是 `robots::Memory`
- 默认 cache backend 是 `robots::cache::Memory`
- 默认 `robots::Memory` 会按 `24h` 的 `cache_ttl` 判断缓存是否过期；调用方也可以通过 `with_cache_ttl(...)` 覆盖，或通过 `without_cache_ttl()` 关闭这层自动过期
- 当 `robots.txt` 临时不可用且当前 origin 没有可用缓存时，默认按 `robots::UnavailablePolicy::AllowAll` fail-open；如果调用方想更保守，可以显式切到 `robots::UnavailablePolicy::DisallowAll`
- 对这类“临时不可用且暂无可用 cache”的结果，默认还会按 `60s` 的 `unavailable_retry_delay` 做短暂退避；在这个窗口内，同一 origin 不会每次请求都重新抓一次 `robots.txt`
- 当前也已提供内置 `robots::cache::File`，用于把 robots policy 持久化到磁盘 JSON 文件；`robots::cache::File::default()` 的路径是 `output/robots-cache.json`
- `robots::Memory` 现在也支持 `with_site_policy(robots::Site::..., robots::SitePolicy::new()...)` 这种显式站点 matcher overlay
- 当前内置的 matcher 是 `robots::Site::origin(...)`、`robots::Site::host(...)` 与 `robots::Site::pattern(...)`
- 这层 overlay 当前可以强制 `AllowAll / DisallowAll`、追加更严格的最小 delay、补充额外 sitemap，以及单独覆盖 unavailable policy
- 多条 matcher 同时命中时，`access` 与 `unavailable_policy` 由更具体 matcher 决定；同一 specificity 下后注册规则优先；`delay` 取更严格值；`sitemaps` 做去重合并
- 如果调用方想保留 `robots::Memory` 这套抓取与判定逻辑、但替换 cache backend，可以继续用 `robots::Memory::with_cache(...)`
- 如果调用方要替换这层策略，可以通过 `Engine::with_robots(...)` 挂自己的实现
- `robots::Robot` 现在除了 `is_allowed(...)`，也可以通过 `check(...)` 返回 `Allow / Disallow / Delay(...)`，并通过 `sitemaps(...)` 读取当前 origin 声明的 sitemap URL
- 如果调用方再显式打开 `Settings::with_robots_sitemap_seeds(true)`，引擎启动时会按 start URL 的 origin 读取 robots 里声明的 sitemap URL，抓取 sitemap / sitemapindex，并把里面的页面 URL 自动转成新的种子请求；当前也支持常见的 `.xml.gz` 压缩 sitemap
- 如果 spider 覆写了 `build_start_requests()`，这些自动种子请求会继续继承对应 start request 的共享请求语义，例如 mode、headers、cookies、timeout、proxy、session
- 这些自动发现出来的种子请求会继续走 `enqueue_request(...)`，所以仍然受 `dedup` 和 `allowed_domains` 过滤；当前默认 `priority / depth` 都保持为 `0 / 0`，也可以通过 `with_robots_sitemap_seed_priority(...)` 和 `with_robots_sitemap_seed_depth(...)` 显式覆盖

当前已补的规则语义：

- 支持 `User-agent`
- 支持 `Allow`
- 支持 `Disallow`
- 支持 `Crawl-delay`
- 支持 `Request-rate`
- 支持 `Sitemap`
- 支持更完整的 `User-agent group` 选择：优先更具体的 agent token；同一 specificity 的 group 会合并规则
- 支持 `*` wildcard 与末尾 `$` end anchor
- 支持按统一 URL 语义解析规则目标：常见的 UTF-8 BOM、absolute URL 和 protocol-relative 规则值都会先归一化，再进入同一套匹配逻辑
- 多个规则同时命中时，优先使用更长路径；同长度时 `Allow` 优先于 `Disallow`
- 如果没有匹配规则，默认允许抓取

当前边界也要明确：

- 只覆盖 HTTP / HTTPS URL；其它 scheme 当前直接放行
- `404 robots.txt` 当前视为允许全部
- `401` / `403 robots.txt` 当前视为拒绝全部
- 其它抓取失败或非成功状态默认走 fail-open，记录日志后允许继续请求；调用方也可以用 `robots::Memory::with_unavailable_policy(robots::UnavailablePolicy::DisallowAll)` 改成更保守的 fail-closed
- stale cache 刷新失败时，当前会优先回退旧缓存，而不是直接把旧 policy 冲掉
- 如果当前 origin 没有可用 cache 且这次抓取临时失败，`robots::Memory` 默认会在 `60s` 的 retry delay 窗口里复用这次 unavailable 决策；调用方也可以通过 `with_unavailable_retry_delay(...)` 覆盖，或通过 `without_unavailable_retry_delay()` 关闭
- sitemap 自动种子当前只走最小 HTTP 抓取和 XML 解析；抓取失败时会记录日志并继续原有 start URL，不会中断整轮爬取
如果只想用内置持久化 cache，可以直接这样挂：

```rust
use halo_spider::engine::Engine;
use halo_spider::robots;
use halo_spider::settings::Settings;

let robots = robots::Memory::new().with_cache(robots::cache::File::default());

let engine = Engine::new()
    .with_robots(robots)
    .with_settings(Settings::default().with_robots_obey(true));
```

如果希望显式调整 robots cache 的 TTL，可以这样写：

```rust
use halo_spider::engine::Engine;
use halo_spider::robots;
use halo_spider::settings::Settings;
use jiff::SignedDuration;

let robots = robots::Memory::new()
    .with_cache(robots::cache::File::default())
    .with_cache_ttl(SignedDuration::from_secs(3600));

let engine = Engine::new()
    .with_robots(robots)
    .with_settings(Settings::default().with_robots_obey(true));
```

如果希望把“robots 临时不可用”的重试窗口调短、调长，或者改成更保守的 fail-closed，可以这样写：

```rust
use halo_spider::engine::Engine;
use halo_spider::robots;
use halo_spider::settings::Settings;
use jiff::SignedDuration;

let robots = robots::Memory::new()
    .with_unavailable_policy(robots::UnavailablePolicy::DisallowAll)
    .with_unavailable_retry_delay(SignedDuration::from_secs(120));

let engine = Engine::new()
    .with_robots(robots)
    .with_settings(Settings::default().with_robots_obey(true));
```

如果你希望对某个站点 matcher 叠加显式站点策略，而不是重写整套 `Robot`，可以这样写：

```rust
use halo_spider::engine::Engine;
use halo_spider::robots;
use halo_spider::settings::Settings;
use jiff::SignedDuration;

let robots = robots::Memory::new()
    .with_site_policy(
        robots::Site::pattern("*.example.com"),
        robots::SitePolicy::new()
            .with_delay(SignedDuration::from_millis(500))
            .with_sitemap("https://example.com/network-sitemap.xml"),
    )
    .with_site_policy(
        robots::Site::host("news.example.com"),
        robots::SitePolicy::new()
            .with_access(robots::SiteAccess::AllowAll)
            .with_unavailable_policy(robots::UnavailablePolicy::DisallowAll),
    );

let engine = Engine::new()
    .with_robots(robots)
    .with_settings(Settings::default().with_robots_obey(true));
```

如果同时希望把 robots 里的 sitemap 自动变成新的种子请求，可以再打开：

```rust
use halo_spider::engine::Engine;
use halo_spider::robots;
use halo_spider::settings::Settings;

let robots = robots::Memory::new().with_cache(robots::cache::File::default());

let engine = Engine::new().with_robots(robots).with_settings(
    Settings::default()
        .with_robots_obey(true)
        .with_robots_sitemap_seeds(true)
        .with_robots_sitemap_seed_priority(10)
        .with_robots_sitemap_seed_depth(1),
);
```

如果调用方只想替换 cache backend，而不是整套 robots 组件，最小边界是实现 `robots::Cache`：

```rust
use halo_spider::error::SpiderError;
use halo_spider::future::BoxFuture;
use halo_spider::robots;

struct MyRobotsCache;

impl robots::Cache for MyRobotsCache {
    fn load<'a>(
        &'a self,
        _origin: &'a str,
    ) -> BoxFuture<'a, Result<Option<robots::CacheEntry>, SpiderError>> {
        Box::pin(async { Ok(None) })
    }

    fn save<'a>(
        &'a self,
        _entry: &'a robots::CacheEntry,
    ) -> BoxFuture<'a, Result<(), SpiderError>> {
        Box::pin(async { Ok(()) })
    }
}

let robots = robots::Memory::new().with_cache(MyRobotsCache);
```

最小自定义 robots 组件可以这样写：

```rust
use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::future::BoxFuture;
use halo_spider::request::Request;
use halo_spider::robots::Robot;
use halo_spider::settings::Settings;

struct AllowOnlyExampleDotCom;

impl Robot for AllowOnlyExampleDotCom {
    fn is_allowed<'a>(
        &'a self,
        request: &'a Request,
        _user_agent: &'a str,
    ) -> BoxFuture<'a, Result<bool, SpiderError>> {
        Box::pin(async move { Ok(request.url.starts_with("https://example.com/")) })
    }
}

let engine = Engine::new()
    .with_robots(AllowOnlyExampleDotCom)
    .with_settings(Settings::default().with_robots_obey(true));
```

## Parser

当前 parser 能力以“已经稳定可用的解析方式”为主：

- CSS
- XPath
- JSON
- XML
- Regex
- Feed
- AI selector

`HTML XPath` 现在已经接入统一执行链路：框架会先用 HTML parser 构建 DOM，再把它规范化成 XML-safe markup，最后交给 `xrust` 执行 XPath；因此 `response.xpath(...)` 与 `response.xml(...)` 现在共享同一套 `one()`、`all()`、`text()`、`html()` 与 `attr()` 语义。

最小用法示例：

```rust
let title = response.xpath("//article//h1").text().one();
let hrefs = response.xpath("//nav//a").attr("href").all();
let section_html = response.xpath("//section[@id='content']").html().one();
```

当前已经落下的最小 query transform 能力：

- `fallback(...)`：查询结果为空时用另一个 query 兜底
- `fallback_many([...])`：按顺序尝试多个 query，返回第一个非空结果
- `field("key")`：从结构化结果里提取对象字段
- `filter_field_present("key")`：只保留指定字段存在且非空的对象结果；适合列表页先筛掉缺字段项
- `filter_field_equals("key", value)`：只保留指定字段等于目标值的对象结果；适合先筛状态、类型、栏目
- `pick_fields([...])`：只保留结构化结果里指定的对象字段；适合先做最小投影，再继续链式读取
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

当前已经覆盖多 query 兜底、结构过滤/投影、数组拉平、结果切片/去重、string transform、URL resolve、embedded JSON parse、number/bool/datetime conversion 与 query-level assertions。

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
- `store` / `scheduler` / `dedup` / `robots` / `http` / `browser`：当前只保留为明确的 future owner 边界，不作为已落地自动装载能力承诺

这样做是为了避免注册表看起来什么都能扩展，但 engine 实际只接了一部分。

## DSL 当前定位

当前阶段，DSL 先后置，不继续扩配置面。

它的定位已经明确：

- 不是另一套运行时
- 不是重新发明一套调度、重试、去重、输出机制
- 而是把代码爬虫已有的底层能力配置化
- `validate` 走共享 validation
- `fetch.request` / `fetch.browser` 走共享 `Request`
- item 输出继续走统一 `pipeline -> store`

也就是说，正确方向应该是：

`代码能力先做实` -> `抽成稳定底层接口` -> `DSL 再映射这些接口`

而不是反过来让 DSL 字段去主导底层模型。
