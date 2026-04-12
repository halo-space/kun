# 技术设计

## 概览

- 这次变更会把底层执行边界重新收口成“request-scoped policy + engine orchestration”模型。
- 对外主心智会进一步收口成“所有 request 级能力都是 middleware，只是生效边界不同”：
  - `Engine` 持有真实 middleware 实例
  - `Step` 持有当前 step 的默认 middleware 配置
  - `Request` 持有当前这一次的 middleware 局部覆盖
  - 三层优先级固定为：`request override > current step default > engine global default`
  - step 之间、request 父子之间都不自动继承 middleware 覆盖
- 目标不是另起一套运行时，而是在当前架构上把已有能力放回正确阶段：
  - `Engine`：worker/process 级编排、scheduler、store、signals、global throughput control
  - `Request`：这一跳请求自己的 middleware 局部覆盖，例如 `dedup / concurrency / interval / rate_limit / auto_throttle / retry_by_status / retry_by_error`
  - `Response`：下载结果对象，以及基于当前 response.url 派生下一条 request 的辅助入口
  - `Config`：顶层全局配置；其中 `engine` 负责 worker/process 级控制，`request` 负责默认 request middleware 来源
- 对外使用方式优先保持 request builder 风格：
  - request-local middleware override 通过 `Request::with_*` 这类链式 builder 暴露
  - 和已有 `with_proxy(...)`、`with_cookie(...)`、`with_timeout(...)` 保持一致
  - 不让调用方退回到手工拼装 free-form runtime map 或大而混杂的全局配置表
- 代码爬虫的主写法会继续向 Scrapy 靠拢：
  - `Request` 作为一等对象承载 `url / method / headers / callback / errback / priority / meta / cb_kwargs / runtime`
  - `Response.follow()` 只负责基于当前 `response.url` 解析相对链接并构造下一条 request
  - callback 输出模型本次保持不变，继续返回 `Output { items, requests }`
- trait 异步实现约束会直接使用 Rust 原生 `async fn in trait`：
  - `Spider`、`Middleware` 以及相关回调接口都优先使用语言原生能力
  - 这次变更不引入 `#[async_trait]`
  - 这样后续 Rust 版本升级时不会额外背上宏兼容与维护风险
- 新的执行边界会明确成三段：
  - `request admission`
    - allowed-domain 检查
    - request dedup
  - `before download attempt`
    - limit / throttle / rate limit / auto throttle
  - `after download attempt`
    - retry by error / retry by status
- 这次不做兼容层。旧的全局 dedup 语义、`dont_filter` 绑死两个边界、`runtime.schedule` 混装下载前 middleware 的路径，会在实现时直接删除或重建。

## Middleware 主模型

- 所有 request 级能力统一抽象成 `Middleware`：
  - 内置能力，如 `dedup / concurrency / interval / rate_limit / auto_throttle / retry_by_status / retry_by_error`
  - 用户自定义能力，如 `custom_header`
- 内置与自定义完全同构：
  - 都通过 `impl Middleware` 实现
  - 都通过 Rust 原生 `async fn in trait` 暴露异步 hook
  - 都由 engine 在固定生命周期边界执行
- 对外 API 分两层：
  - 内置快捷 builder：
    - `with_dedup(...)`
    - `with_concurrency(...)` / `with_interval(...)` / `with_rate_limit(...)` / `with_auto_throttle(...)`
    - `with_retry_by_status(...)` / `with_retry_by_error(...)`
  - 通用 builder：
    - `with_middleware::<T>(cfg)`
    - `skip([DEDUP, CONCURRENCY, INTERVAL, RATE_LIMIT, ...])`
- `skip([...])` 是统一禁用入口：
  - 不再为每个内置 middleware 暴露独立 `disable_xxx()`
  - 内置 middleware 会导出稳定常量名，如 `DEDUP / CONCURRENCY / INTERVAL / RATE_LIMIT / AUTO_THROTTLE / RETRY_BY_STATUS / RETRY_BY_ERROR`

## 生命周期、Flow 与 Context

- `Flow` 不按“一个 hook 一个类型”拆，而是按对象生命周期拆成 4 组：
  - `flow::Enqueue`
    - `Continue`
    - `Drop { reason }`
  - `flow::Download`
    - `Continue`
    - `Drop { reason }`
    - `Delay { reason, millis }`
    - `Retry { reason, backoff }`
  - `flow::Parse`
    - `Continue`
    - `Drop { reason }`
  - `flow::Item`
    - `Continue`
    - `Drop { reason }`
- hook 也按生命周期拆，而不是继续沿用容易混淆的 `before_request / before_response / on_exception`：
  - `before_enqueue -> flow::Enqueue`
  - `after_enqueue -> Result<(), SpiderError>`
  - `before_download -> flow::Download`
  - `after_download -> flow::Download`
  - `download_error -> flow::Download`
  - `before_parse -> flow::Parse`
  - `parse_error -> flow::Parse`
  - `after_parse -> Result<(), SpiderError>`
  - `before_item -> flow::Item`
  - `after_item -> Result<(), SpiderError>`
- 这里刻意把“控制流决策”和“finally/收尾 hook”分开：
  - `after_enqueue / after_parse / after_item` 这类 hook 不再返回 flow，只做日志、清理、状态释放、埋点等副作用
  - `after_download` 仍然保留 flow，是因为它代表“下载成功拿到 response 后”的控制点，`retry_by_status` 等能力需要在这里决定是否 `Retry`
- `Download` 虽然是一个共享枚举，但不同 hook 的允许分支仍然由 engine 校验：
  - `before_download`
    - 允许 `Continue / Drop / Delay`
    - 不允许 `Retry`
  - `after_download`
    - 允许 `Continue / Drop / Retry`
    - 不允许 `Delay`
  - `download_error`
    - 允许 `Continue / Drop / Retry`
    - 不允许 `Delay`
- `Parse` 与 `Item` 都保持窄模型：
  - `before_parse / parse_error`
    - 只允许 `Continue / Drop`
  - `before_item`
    - 只允许 `Continue / Drop`
  - 本次不为 item 额外设计 `Delay / Retry`
- `Delay` 的语义固定为：
  - 当前 task 返回 scheduler delayed bucket
  - 不阻塞当前 worker，不占住 inflight 执行时间
  - 不计入 retry 次数
- `Retry` 的语义固定为：
  - 这是一次下载失败或 retryable response 之后的重试
  - 继续沿用原 request 的有效 middleware 上下文
  - retry 计数、原因和 backoff 只由 retry 路径维护
- `Context` 也不再按“每个 hook 一个 context”拆，而是同样按对象生命周期拆：
  - `context::Enqueue`
    - 持有 request 与 spider/task 级最小上下文
  - `context::Download`
    - 持有 request、task_id、attempt、spider 等下载阶段稳定信息
  - `context::Parse`
    - 持有 request + response + spider/task 级解析上下文
  - `context::Item`
    - 持有 item，以及必要时关联 request / response / spider/task 上下文
- `response`、`error` 这类事件数据不强塞进大一统 context，而是作为 hook 参数单独传入：
  - `after_download(&mut context::Download, &mut Response) -> flow::Download`
  - `download_error(&mut context::Download, &SpiderError) -> flow::Download`
  - `parse_error(&mut context::Parse, &SpiderError) -> flow::Parse`
- 命名规则固定为：
  - `src/engine/context.rs` 与 `src/engine/flow.rs` 中的类型名本身不带 `Context / Flow` 后缀
  - 调用方默认通过模块命名空间区分，例如：
    - `use crate::engine::{context, flow};`
    - `context::Download`
    - `flow::Download`
  - 只有在个别局部作用域确实需要时，才使用 `as DownloadContext / as DownloadFlow`

## Request Middleware Override 模型

- request 不保存真正的 middleware 实例，只保存本次需要的稀疏覆盖：
  - `Use(options)`
  - `Skip`
- `options` 是小而局部的 `MiddlewareOptions`
  - 只表示当前 middleware 这一项的配置
  - 不做跨 rules、跨 step 的全局大表
- 同名 middleware 的配置合并规则：
  - 不做字段级 merge
  - 只做整项替换
  - 最后一次显式写入生效
- 执行时的有效配置解析顺序固定为：
  - 先看 request override
  - 再看当前 step default
  - 最后看 engine global default
  - 不看父 request，也不看前一个 step

## Typed Config 与 Options 编解码

- 用户侧始终使用强类型配置对象：
  - `RetryConfig`
  - `LimitsConfig`
  - `DedupConfig`
  - 以及用户自己的 `HeaderConfig`、`CustomXxxConfig`
- 底层统一落成 `MiddlewareOptions`
- 建议保留两个编解码 trait：
  - `IntoMiddlewareOptions`
  - `FromMiddlewareOptions`
- 这样 DSL 与代码爬虫可以共享同一套底层结构：
  - 代码 builder 先编码成 `MiddlewareOptions`
  - middleware 实现执行时再从 `MiddlewareOptions` 解回强类型配置

## 模块影响

- `src/request.rs`
  - 把当前 free-form `RuntimeOverride` 收口成 request-local middleware override 入口。
  - 拆分 `dont_filter`，改成分别控制 dedup bypass 和 domain-filter bypass 的显式字段。
  - 对齐 Scrapy 风格的一等请求对象，补齐稳定字段与 builder，例如 callback / errback / priority / cb_kwargs / meta / middleware。
  - 提供更自然的请求构造 sugar，例如 `Request::to(...)`、`with_meta_map(...)`、`with_retry(...)`、`skip([...])` 这类直接围绕“下一条 request”表达的 API。
- `src/response.rs`
  - 保持 `Response` 作为下载结果对象，不让 runtime policy 误挂到 `Response` 上。
  - 对齐 `urljoin / follow / follow_all / meta / cb_kwargs` 这类与当前 request 关联紧密的辅助语义。
  - `follow()` 只负责 URL 派生和核心请求语义继承，不承担 request runtime 本体。
- `src/spider.rs`
  - 保持当前 callback 返回 `Output { items, requests }` 的主模型不变。
  - 继续收口 callback / errback / dispatch 与 `Request` 一等对象之间的边界。
- `src/runtime.rs`
  - 重新整理当前 `schedule / retry / dedup` 这些配置来源，让外部 surface 不再强依赖 free-form runtime map。
  - `schedule` 这一节不再继续承载下载前 middleware 的主语义；request/step/global middleware override 成为真正稳定的底层模型。
- `src/runtime/compile.rs`
  - 从“把 runtime.schedule 编译成 middleware”改成“把 request runtime 中真正属于 download attempt 的下载前 middleware / retry middleware 编译到对应执行边界”。
  - dedup 不再作为全局 engine 组件语义继续存在。
- `src/engine.rs`
  - 去掉“engine 全局 dedup 组件决定所有 request 是否入队”这条主路径。
  - 保留 worker/process 级 global concurrency 与 per-domain concurrency，但把它们明确定义为 engine throughput control，而不是 request 级下载前 middleware。
  - 持有真实 middleware 实例，并统一 start / manual enqueue / follow / retry 的 effective middleware 计算入口。
  - 继续把 spider callback 返回的 `Output { items, requests }` 统一收口回 engine。
- `src/engine/task.rs`
  - 把 request admission、before-download、after-download 三个执行边界明确拆开。
  - retry task 继续保留 task identity，同时保留原 request 的有效 runtime。
  - spider callback 返回的 request 会在返回 engine 后统一进入 admission 边界，而不是在 callback 内即时下载。
- `src/engine/context.rs`
  - 不再继续承载大一统 `EngineContext`；会按 `enqueue / download / parse / item` 四类生命周期上下文重建。
- `src/engine/flow.rs`
  - 不再继续使用一个总 `Flow` 覆盖所有 hook；会改成 `Enqueue / Download / Parse / Item` 四组 flow。
- `src/middleware/traits.rs`
  - middleware hook 会从旧的 `before_request / before_response / on_exception` 收口成更直白的生命周期命名，并与四组 flow 对齐。
- `src/middleware/chain.rs`
  - chain dispatch 会从统一 `Flow` 转成按 `enqueue / download / parse / item` 分发，减少运行时兜底判断。
- `src/middleware/dedup/mod.rs`
  - 从“全局 yes/no gate”改成“按 effective request dedup policy 做 admission 决策”的底层边界。
  - built-in `Memory / Bloom / Noop` 需要按新边界重建，不强保留旧 trait 语义。
- `src/middleware/dedup/*`
  - `Memory / Bloom / Noop` 需要围绕新的 dedup policy 契约重写。
- `src/middleware/concurrency.rs`
  - 当前实现是 middleware instance 内部计数器；这次会改成围绕显式 limit bucket 生效，而不是 anonymous chain-local state。
- `src/middleware/interval_gate.rs`
  - 当前实现是 middleware instance 内部 `next_allowed_at`；这次会改成 keyed bucket state。
- `src/middleware/rate_limit.rs`
  - 当前实现是 middleware instance 内部滑窗；这次会改成 keyed bucket state。
- `src/middleware/auto_throttle.rs`
  - 继续保留在 download-before 边界，但要和新的 request limit bucket 语义对齐。
- `src/settings.rs`
  - 回收挂错阶段的能力：`download_delay`、`auto_throttle*`、`retry_*` 不再假装是 engine 自己的运行阶段，而是“默认 request runtime”来源。
- `examples/custom_dedup.rs`
  - 按新的 request-scoped dedup 模型改写示例。
- `examples/concurrency_control.rs`
  - 按新的下载前 middleware / worker controls 边界改写示例。
- `examples/period_xml_spider.rs`
  - 作为代码爬虫主示例，继续体现 `Request + callback + meta` 透传这条主写法。
- `docs/capabilities.md`
  - 重新说明 engine、request runtime、scheduler admission、download attempt 的边界。
- `docs/guide/getting-started.md`
  - 移除旧的“dedup 是 engine 全局组件”表述。
- `docs/guide/scheduler-and-runtime.md`
  - 重新拆清 scheduler、worker/process control、request runtime 三层语义。
- `openspec/specs/runtime-engine/spec.md`
  - 增量声明 request-scoped execution policy 与显式执行边界。
- `openspec/specs/spider-api/spec.md`
  - 增量声明 `Request` 的新 runtime / bypass 语义。

## 关键决策

- Runtime / middleware 影响：
  - `dedup` 不再挂成“全局 engine 组件”。
  - `concurrency / interval / rate_limit / auto_throttle` 仍然属于 download 前能力，但不能再依赖 middleware instance 本地状态来表达共享 bucket。
  - `retry` 继续保留在 download attempt 之后，但它的输入要来自 effective request runtime，而不是只吃全局 settings。
  - `auto_throttle` 继续保留为 request limit 的一种特殊实现，不再伪装成 `schedule`。
  - engine 外层的 `concurrent_requests` 与 `concurrent_requests_per_domain` 继续存在，但它们明确只表示 worker/process throughput control。
  - request/step/global 三层 middleware 配置不做跨 step、跨父子 request 的自动继承。
- 对外 API 影响：
  - `Request.runtime` 不再继续作为对外主入口；request-local middleware override 会成为更稳定的用户可见模型。
  - request-local middleware override 会优先以 builder API 暴露，例如 `with_dedup(...)`、`with_interval(...)`、`with_retry_by_status(...)`，并和 `with_proxy(...)`、`with_cookie(...)` 这类已有 request builder 保持一致。
  - 通用禁用入口统一为 `.skip([...])`，而不是为每个内置 middleware 分别暴露独立 `disable_xxx()`。
  - 内置 middleware 导出稳定常量名，供 `skip([...])` 使用。
  - 代码爬虫继续通过 `Output { items, requests }` 返回下一批工作；这次不改成 yield 风格 helper。
  - `Response.follow(...)` 仍然保留，但它只负责相对 URL 补全与便捷构造，不再承担 request runtime 主语义；绝对 URL 场景优先直接从 `Request::new(url)` 或 `url.to(...)` 起手。
  - `dont_filter` 会被拆掉，不再继续同时绕过 dedup 和 allowed-domain。
  - 旧的“engine 全局 dedup 激活”路径会被删除或重建为更符合新语义的 API，不继续保留兼容层。
  - 顶层 `Config` 上那些实际上属于 request policy defaults 的 builder，会按新的边界重新定义，并统一写回 `Config.request.middleware`。
  - `Spider` / `Middleware` 的异步 trait 实现统一使用 Rust 原生 `async fn in trait`，不依赖 `#[async_trait]`。
- Plugin 或 DSL 影响：
  - plugin 仍然只负责 middleware 装配，不额外承担 request policy lifecycle 编排。
  - DSL 这次不实现，但新的底层 request runtime 边界会成为后续 DSL `engine.*` 的唯一落点。

## Spider Callback 与 Request/Response 模型

- `Request` 是真正的一等对象：
  - callback、errback、priority、meta、cb_kwargs、runtime 都挂在 `Request` 上。
  - request runtime 永远跟着 request 走，而不是跟着 callback 名或 `Response` 走。
- `Response` 是下载结果对象：
  - `Response.request` 表示触发这次下载的 request。
  - `Response.meta` / `Response.cb_kwargs` 作为 `Response.request` 上下文的 shortcut 暴露给 spider 代码。
  - `Response.follow(...)` / `follow_all(...)` 只是基于当前 response.url 派生下一条 request 的 helper。
- 代码爬虫 callback 的主写法：
  - 用户在 callback 中直接构造下一条 `Request` 或 `Item`。
  - callback 继续返回 `Output { items, requests }`，不改成 yield 风格。
  - engine 在 callback 返回后统一接管 output 中的 request / item，并继续走 admission -> download -> retry 这些固定边界。

## 自定义能力扩展点

- `dedup / concurrency / interval / rate_limit / auto_throttle / retry_by_status / retry_by_error` 的声明入口固定在 `Request::with_*` builder 上。
- 用户自定义 middleware 与内置 middleware 完全同构：
  - 也是 `impl Middleware`
  - 也是通过异步 hook 实现具体逻辑
  - 也是通过 `with_middleware::<T>(cfg)` 与 `skip([...])` 接入 request
- 设计上保留三层：
  - engine 负责持有与执行真实 middleware 实例，并提供全局默认配置
  - step 负责当前 step 的默认 middleware 配置
  - request 负责当前这一跳的局部覆盖，并在序列化后继续跟着 request 走
- 这样既能支持全局默认值，也能支持 step 默认值与单条 request 在 callback 中显式覆盖。

## 当前错位能力清单

- `Engine::with_dedup(...)`
  - 现在把 request dedup 表达成 engine 全局开关，和“每条 request 自己决定是否 dedup”冲突。
- `Request.dont_filter`
  - 现在同时绕过 dedup 与 allowed-domain，是两个执行阶段的硬耦合。
- `Request.runtime`
  - 当前只有字段，没有真实执行语义，而且还是 free-form map，不适合作为稳定底层边界。
- `runtime.schedule`
  - 名字像 scheduler 配置，实际却承载 `interval / rate_per_minute / auto_throttle` 这些 download-before middleware。
- 旧的 `download_delay / retry_times / retry_http_codes / auto_throttle*` 全局 builder
  - 原来挂在全局设置上，但语义其实更接近“默认 request runtime”；现在统一由顶层 `Config.request.middleware` 承载。
- `concurrency / interval_gate / rate_limit`
  - 执行阶段是对的，作用域却不对；现在状态绑在 middleware instance 上，不能稳定表达共享 limit bucket。
- `Response.follow(...)`
  - 当前容易被误用成 request 主入口；实际上它应该只是相对 URL 派生 helper，不能混淆成 runtime 或发射 API。

## 验证方式

- 单元测试覆盖：
  - effective request runtime merge
  - start / manual enqueue / follow / retry 都能拿到正确 runtime
  - request dedup 只在配置时生效
  - retry request 不会被原始 dedup policy 误判成重复
  - `skip_dedup` 与 `skip_domain_filter` 分离
  - `Request` / `Response` 的主语义与 shortcut 对齐预期
  - spider callback 返回 request / item 后，engine 能正确收集并继续执行
  - 同 bucket 下载前 middleware 共享状态，不同 bucket 相互隔离
- 集成测试覆盖：
  - `download_delay`、`auto_throttle`、`retry_*` 作为默认 request runtime 的行为
  - `custom_dedup` 与 `concurrency_control` 示例行为
  - `period_xml_spider` 等代码爬虫示例的回调写法与下一跳 request runtime 传递
- 文档验证：
  - `docs/capabilities.md`
  - `docs/guide/getting-started.md`
  - `docs/guide/scheduler-and-runtime.md`
- 运行命令：
  - `cargo fmt --all`
  - `cargo test -q`
