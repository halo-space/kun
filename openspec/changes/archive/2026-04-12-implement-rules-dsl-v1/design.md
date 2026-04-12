# 技术设计

## 概览

- 这次变更以重写 `src/rules/*` 为主；`src/engine.rs`、`src/engine/task.rs` 等底层模块不作为主动改造目标，但如果 `src/runtime.rs`、`src/runtime/compile.rs` 在 `rules` 迁移后只剩过渡职责，会在本次 change 里一起删除并清理引用。
- 目标不是让 DSL 反过来推动底层改造，而是让新的 DSL v1 在 `rules` 编译阶段尽量贴合现有底层能力。
- 对外兼容面继续保持：
  - `Spider::rules()`
  - `rules::Config::{local, inline}`
  - `Engine::run()` 自动加载 rules
  - `Spider::dispatch()` 在 callback 与 DSL step 之间分发
- 内部实现会重建三层：
  - authored schema：对应 DSL 文档里的 `spider / engine / sinks / seeds / steps`
  - compiled plan：面向运行时执行的 `CompiledStep`、`CompiledFollow`、`CompiledOutput` 等结构
  - runner：负责执行 `fields / bind / follow / output`
- 运行时仍然复用现有统一主链：
  - request 继续走 `Request`
  - response 继续走 `Response`
  - item 继续走 `pipeline -> validator -> store`
- 对 DSL `engine` 配置的处理策略是：
  - 能在 `rules` 编译阶段直接映射到现有底层语义的，就在 `src/rules/*` 内完成 lowering
  - 当前底层没有接线的位置，不在这次 change 里直接补底层代码，而是单独列出 gap

## 模块影响

- `src/rules/schema.rs`
  - 重建 DSL authored schema 与 compiled plan
  - 新增 `CompiledFollow`、`CompiledOutput`、`ValueExpr` 等结构
- `src/rules/compile.rs`
  - 重写编译流程
  - 从 `Value` 解析新 DSL
  - 校验 step / sink / engine registry 引用关系
  - 把 authored schema 编译成运行时 plan
  - 把 DSL `engine` 配置尽量 lowering 到现有底层已支持的 runtime 形态
- `src/rules/validate.rs`
  - 从“旧字段存在性检查”改成新 DSL 结构校验
  - 删除历史兼容结构约定
- `src/rules/run.rs`
  - 重写 step 执行器
  - 支持 `fields / bind / follow / output / meta / allow_url_pattern`
  - 支持统一值模型和最小 transforms 集合
- `openspec/specs/rules-dsl/spec.md`
  - 补充新的 DSL v1 需求差异

本次变更以 `src/rules/*` 为主；如果实现过程中确认某个能力无法在 `rules` 内贴合到底层，就记录为后续集成问题。唯一明确的跨模块清理项是：当 `rules` 不再依赖 runtime 过渡层后，删除 `src/runtime.rs` 与 `src/runtime/compile.rs`。

## 关键决策

- Runtime / middleware 影响：
  - 不主动扩展新的底层 runtime 结构。
  - DSL `engine` 配置在 `rules` 编译阶段尽量映射到当前底层已经认得的能力形态。
  - `engine.limit` 不直接推动新增底层结构；优先在 `rules` 内找到可复用的现有限流接线方式。
  - 如果 `rules` 完成迁移后已经不再需要 `src/runtime.rs`、`src/runtime/compile.rs` 这层中间表达，就在本次 change 里直接删除，而不保留兼容过渡层。
  - `follow` 上的局部 `engine` 配置优先通过编译期降级/展开解决，例如为带局部 engine 配置的 follow 生成独立 compiled step 变体，而不是要求引擎支持 request-level runtime override。
- 对外 API 影响：
  - `Spider::rules()` 与 `rules::Config` 入口保持不变。
  - `src/rules` 内部历史 schema 不再作为稳定内部 API 继续维护。
  - 历史 DSL 字段面直接删除，不再保留兼容或迁移层。
- Engine 与 rules DSL 影响：
  - step 只描述“当前页面如何继续流转”，不再承载额外的分叉语义。
  - follow 的 `item` 代表子链路节点作用域，不是批处理语法；runner 需要在这个作用域内计算 `request` 和 `meta`。
  - 顶层 `seeds` 会接入引擎起始请求生成；当 `rules.seeds` 非空时，它作为实际起始请求来源。
- Output 与 store 影响：
  - 本次实现继续沿用统一 `item -> pipeline -> validator -> store` 主链。
  - 不在 engine 内再增加第二套独立 sink runtime。
  - DSL `output.validate` 直接映射到共享 validation。
  - DSL `sinks` / `output.sinks` 会解析为目标 store 路由，并复用 engine store 注册表。
- Plugin 或 DSL 影响：
  - plugin 装载机制不变。
  - middleware 仍然通过既有 registry/build 链路构建。
  - 这次只让 DSL 更准确地复用共享 engine 能力，不引入新的 plugin 类型。

## 当前集成状态

- `output.validate.required` / `fields` 已接入 step validator；`output.validate.rule` 已从当前 DSL 中移除。

## 验证方式

- 为 `src/rules/compile.rs` 增加单元测试，覆盖：
  - 新 DSL 顶层结构解析
  - step / sink / engine registry 引用校验
  - 非 v1 结构已被删除
- 为 `src/rules/run.rs` 增加单元测试，覆盖：
  - `fields`
  - `bind`
  - `follow.item + meta`
  - `output.item + validate`
  - `allow_url_pattern`
- 为 `src/rules/compile.rs` 增加测试，覆盖局部 `engine` 配置如何被 lowering 到现有底层可运行的 compiled step 计划。
- 验证命令以 `cargo test` 为主；阶段末尾至少保持 `rules` 相关测试通过。
