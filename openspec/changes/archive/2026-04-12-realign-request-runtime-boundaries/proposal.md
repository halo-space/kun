# 变更提案

## 为什么做

- 当前底层把“进程级 engine 控制”和“单条 request 的执行策略”混在了一起，最明显的问题有：
  - `dedup` 被建模成 `Engine::with_dedup(...)` 这种全局组件，调用方很难表达“列表页 request 用一套去重，详情页 request 用另一套，某一跳不去重”。
  - `concurrency / interval / rate_limit / auto_throttle` 曾经一部分挂在旧的全局并发/速率设置入口，另一部分又藏在 `runtime.schedule -> middleware`，边界不清晰。
  - `retry` 是 request 级语义，但旧的默认入口仍然主要挂在全局 retry 设置上。
  - `Request.runtime` 字段已经存在，但目前是 free-form 值模型，而且没有真正接入执行链路。
  - `dont_filter` 同时绕过 dedup 和 allowed-domain 检查，把两个不同阶段的能力绑死在一个字段上。
  - 当前 `Request` / `Response` 表面上已经有 callback、meta、follow 这些概念，但实际 API 还没有收口成稳定的一等请求对象模型，容易把 `response.follow(...)`、`callback`、`runtime`、输出收集这几层混在一起。
- 这会直接影响代码爬虫、DSL 以及后续 runtime 设计：
  - 代码爬虫很难给不同 request 配不同的 dedup、下载前控制与重试策略。
  - DSL 很难稳定下沉到底层，因为底层并没有真正的 request-scoped `engine` 执行边界。
  - 文档里虽然已经区分了 `engine / runtime / schedule`，但真实代码执行边界仍然“拧巴”。
- 对 `halo-spider` 来说，更合理的模型应该是：
  - `Engine` 负责总编排和 worker/process 级控制。
  - `Engine` 持有真实中间件实例，并在固定生命周期边界执行它们。
  - `Request` 负责自己这一跳的中间件局部覆盖，而不是再暴露一套抽象但模糊的 free-form runtime map。
  - `Step` 负责当前 step 的默认中间件配置。
  - `Response` 负责承载下载结果，并提供 `urljoin/follow/follow_all` 这类“基于当前响应方便地产生下一条 request”的辅助能力，但不承担 request runtime 本体。
  - `dedup`、下载前 middleware、重试 middleware 统一抽象成 middleware，只是在不同边界执行：
    - `dedup`：request admission 阶段
    - `concurrency / interval / rate_limit / auto_throttle`：每次 download attempt 前
    - `retry_by_status / retry_by_error`：download 失败或响应返回后
  - middleware 的最终生效优先级固定为：
    - request 局部覆盖
    - 当前 step 默认配置
    - engine 全局默认配置
  - request 之间、step 之间都不自动继承 middleware 覆盖；`step1` 的配置不会流到 `step2`。

## 范围

- 会影响哪些 capability specs：
  - `openspec/specs/runtime-engine/spec.md`
  - `openspec/specs/spider-api/spec.md`
- 会影响哪些模块 / 示例：
  - `src/request.rs`
  - `src/response.rs`
  - `src/spider.rs`
  - `src/runtime.rs`
  - `src/runtime/compile.rs`
  - `src/engine.rs`
  - `src/engine/context.rs`
  - `src/engine/flow.rs`
  - `src/engine/task.rs`
  - `src/middleware/traits.rs`
  - `src/middleware/chain.rs`
  - `src/middleware/dedup/mod.rs`
  - `src/middleware/dedup/*`
  - `src/middleware/concurrency.rs`
  - `src/middleware/interval_gate.rs`
  - `src/middleware/rate_limit.rs`
  - `src/middleware/auto_throttle.rs`
  - `src/settings.rs`
  - `examples/custom_dedup.rs`
  - `examples/concurrency_control.rs`
  - `examples/period_xml_spider.rs`
  - `docs/capabilities.md`
  - `docs/guide/getting-started.md`
  - `docs/guide/scheduler-and-runtime.md`
- 预期带来哪些用户可见结果：
  - 单条 request 可以显式携带自己的 `dedup / concurrency / interval / rate_limit / auto_throttle / retry_by_status / retry_by_error` 配置。
  - 列表页 request、详情页 request、重试 request 可以使用不同的执行策略，而不是共享一套 engine 全局 dedup。
  - request-local middleware override 会优先以 builder 风格暴露，例如 `with_dedup(...)`、`with_interval(...)`、`with_retry_by_status(...)`，并和已有 `with_proxy(...)`、`with_cookie(...)` 这类 request builder 保持同一套心智模型。
  - 内置 middleware 会导出稳定常量名，例如 `DEDUP / INTERVAL / RETRY_BY_STATUS`，供 `.skip([...])` 这类通用 API 使用，避免用户手写字符串。
  - 用户自定义 middleware 与内置 middleware 完全同构：都通过 `impl Middleware` 实现异步 hook，再由 request 通过 `.with_middleware::<T>(cfg)` 或 `.skip([...])` 控制本次是否生效。
  - `Request` / `Response` 的主写法会更贴近 Scrapy 的一等对象模型：`Request` 自己携带 callback、meta、cb_kwargs、priority、runtime 等请求语义；`Response.follow(...)` 只作为 URL 补全和便捷构造器。
  - `dedup`、下载前 middleware、重试 middleware 的执行阶段会更清晰，后续 DSL 可以直接往这套底层边界编译。
  - 明显不合理的旧路径会直接删除，不再加兼容层继续维持“双语义”。

## 非目标

- 这次不顺带重写 `rules DSL` 编译与运行逻辑；DSL 只作为后续要对齐的消费者，不纳入当前实现范围。
- 这次不改 parser / response query / pipeline / store 的核心语义。
- 这次不新增结果层 item dedup；范围只收口在 request 层执行策略。
- 这次不调整代码爬虫 callback 的 `Output { items, requests }` 输出模型；它继续作为当前代码爬虫的稳定接口。
- 这次不追求完整复刻 Scrapy 的全部对象模型；目标是把 request runtime 与 `Request/Response` 主心智收口到更稳定、可扩展的底层边界。
- 这次不保留 `with_dedup`、`dont_filter`、`runtime.schedule` 这套旧模型的兼容适配层；如果确认不合理，直接删除或重建。

## 风险

- 是否存在兼容性或迁移风险：
  - 存在。这次会直接调整 request/runtime/engine 的公开边界，示例、文档和可能的外部调用代码都需要迁移。
- 是否存在 runtime、middleware 或 plugin 相关风险：
  - 存在。`dedup`、下载前 middleware、重试 middleware 的执行阶段和状态边界会被重新梳理，相关 middleware、测试和文档都需要同步更新。
