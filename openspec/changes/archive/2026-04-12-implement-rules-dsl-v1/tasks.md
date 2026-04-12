# 任务清单

- [x] 1.1 复核 `implement-rules-dsl-v1` 的 proposal、design、`specs/rules-dsl/spec.md` 与 `tasks.md`，确认这次 change 以 `src/rules/*` 为主，并在 `rules` 不再依赖 runtime 过渡层后删除 `src/runtime.rs`、`src/runtime/compile.rs` 与相关引用；`src/engine.rs`、`src/engine/task.rs` 仍不作为本次主动改造目标。
- [x] 1.2 在 `src/rules/schema.rs` 中重建新的 authored schema 与 compiled plan，覆盖 `spider / engine / sinks / seeds / steps` 顶层结构，以及 step 内的 `fields / bind / follow / output / meta` 与统一值模型。
- [x] 1.3 在 `src/rules/validate.rs` 中重写 rules 校验逻辑，覆盖 step / seed / sink / engine registry 的结构校验与引用校验；历史结构不再作为兼容输入继续支持。
- [x] 1.4 在 `src/rules/compile.rs` 中重写编译流程，把新的 DSL v1 编译成运行时 plan，并保留 `rules::Config::{local, inline}` 与 `rules::load()` 外部入口不变。

- [x] 2.1 在 `src/rules/run.rs` 中重写 step runner，按 `fields -> bind -> follow / output` 的顺序执行，并支持统一值模型求值。
- [x] 2.2 在 `src/rules/run.rs` 中实现单条链路语义：每次 follow 只围绕当前命中的一个 URL 继续流转，并把该 URL 在上游解析出的 `meta` 与当前请求绑定。
- [x] 2.3 在 `src/rules/run.rs` 中实现 `allow_url_pattern` 过滤、`output.item` 组装，以及 `output.validate` 到共享 validation 的桥接，并保持 item 继续走统一 `pipeline -> store` 主链。

- [x] 3.1 在 `src/rules/compile.rs` 中梳理 `engine.dedup / engine.concurrency / engine.interval / engine.rate_limit / engine.auto_throttle / engine.retry_by_status / engine.retry_by_error` 如何贴合现有底层能力：能在 `rules` 内完成 lowering 的直接接入，不能接入的明确记录为 integration gap。
- [x] 3.2 在 `src/rules/schema.rs` 与 `src/rules/compile.rs` 中保留 `seeds`、`sinks` 等 authored 配置面的建模与编译结果，并把 `seeds` 接到起始请求生成、把 `output.sinks` 接到统一 store 路由。
- [x] 3.3 当 `src/rules/compile.rs` 已直接产出新 request/middleware 主模型、不再依赖 runtime 过渡层后，删除 `src/runtime.rs`、`src/runtime/compile.rs` 与 `src/lib.rs` 中相关导出，并补测试确认 `rules` 主链仍可工作。

- [x] 4.1 为 `src/rules/compile.rs`、`src/rules/run.rs`、`src/rules/validate.rs` 补单元测试，覆盖新 DSL 主链、非 v1 结构不再支持、单条链路 `meta` 绑定，以及 engine registry 引用校验。
- [x] 4.2 如果实现边界与当前 DSL 指南存在差异，更新 `docs/guide/rules-dsl.md` 与相关文档，使文档描述与真实实现一致，并把当前仍依赖底层改造的点列为后续问题。
- [x] 4.3 运行验证命令并记录结果：`cargo fmt --all`、`cargo test -q rules`
