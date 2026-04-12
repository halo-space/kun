# 任务清单

- [x] 1.1 更新 `realign-request-runtime-boundaries` 的 proposal、design 与 spec 增量文档，确认这次变更以 request-scoped `dedup / download-before middleware / retry middleware` 为目标，并把 `Request/Response` 主模型一起收口，不保留旧的全局兼容语义。
- [x] 1.2 在 `src/request.rs` 中补齐 request-local bypass 收口的剩余边界，明确 dedup / domain 两条显式语义，并保持 middleware override builder 与 `with_proxy(...)`、`with_cookie(...)` 这类 request builder 的一致性。
- [x] 1.3 在 `src/request.rs` 中补齐一等请求对象所需的稳定字段与 builder，至少覆盖 callback / errback / priority / meta / cb_kwargs / middleware override 的主路径，并提供更自然的 `Request::new(url)` / `to(...)` / `skip([...])` 类写法。
- [x] 1.4 在 `src/response.rs` 中重建 `Response` 的辅助职责，明确 `meta / cb_kwargs` shortcut、`urljoin / follow / follow_all` 的语义边界，不让 request runtime 错挂到 `Response` 上。
- [x] 1.5 重新梳理默认 request runtime config 的承载位置，清理旧任务中 `src/runtime.rs` / `schedule` 的过时描述，并继续收口到顶层 `Config` 与 request middleware 默认配置边界。
- [x] 1.6 在 `Spider`、`Middleware` 与相关回调 trait 改造中统一使用 Rust 原生 `async fn in trait`，不引入 `#[async_trait]` 宏依赖。

- [x] 2.1 在 `src/settings.rs` 中把 engine worker/process control 与 default request runtime 拆开，回收挂错阶段的 runtime helper 语义。
- [x] 2.3 在 `src/engine.rs` 中统一 start request、manual enqueue、follow request、retry request、spider callback 返回 request 的 effective middleware 计算，并固定成 `request > current step > engine global` 的解析顺序。
- [x] 2.4 在 `src/engine/task.rs` 中把 admission、before-download、after-download 三个执行边界显式拆开，并固定成“dedup admission -> download-before middleware -> retry middleware”的顺序。

- [x] 3.1 在 `src/middleware/dedup/mod.rs` 与 `src/middleware/dedup/*` 中重写 dedup 边界，让它按 effective request dedup policy 工作，而不是继续作为 engine 全局 yes/no 组件。
- [x] 3.2 在 `src/engine.rs` / `src/engine/task.rs` 中删除旧的全局 dedup 激活路径，并确保 internal retry 不会被 admission dedup 误拒绝。
- [x] 3.3 在 `src/request.rs`、`src/engine/task.rs` 与相关测试中拆掉 `dont_filter`，改成显式的 dedup/domain 两条 bypass 语义。

- [x] 4.1 在 `src/runtime/compile.rs`、`src/middleware/concurrency.rs`、`src/middleware/interval_gate.rs`、`src/middleware/rate_limit.rs` 与 `src/middleware/auto_throttle.rs` 中重构 download-before middleware 执行路径，改成显式 keyed bucket state。
- [x] 4.2 在 `src/engine/task.rs` 中让 retry by error / retry by status 使用 effective middleware 配置，区分 `Delay` 与 `Retry`，并在 delayed retry 后保持原 request middleware 上下文。
- [x] 4.3 为 admission dedup、download-before bucket middleware、retry 边界，以及 spider callback 返回的 request / item 收集与执行补单元测试与集成测试。

- [x] 5.1 更新 `examples/custom_dedup.rs`、`examples/concurrency_control.rs` 与 `examples/period_xml_spider.rs`，使示例符合新的 request runtime 边界。
- [x] 5.2 更新 `docs/capabilities.md`、`docs/guide/getting-started.md` 与 `docs/guide/scheduler-and-runtime.md`，把 engine/request/runtime 的新边界写清楚。
- [x] 5.3 运行验证命令并记录结果：`cargo fmt --all`、`cargo test -q`（2026-04-12 通过，`cargo test -q` 结果：530 个单元测试通过，doctest 全部通过，额外 9 个 doctest/集成项为 ignored）
