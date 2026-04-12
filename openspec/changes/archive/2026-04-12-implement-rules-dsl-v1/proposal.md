# 变更提案

## 为什么做

- 当前 `src/rules` 的结构仍然和我们确认的新 DSL v1 单链路模型明显脱节。
- 现有实现不仅字段模型不一致，运行时桥接也不完整：
  - `Spider::rules()` 入口还在，但编译结果无法表达 `seed -> step -> follow -> output` 这条新链路。
  - 顶层 `engine` 配置、局部 `follow.engine` 引用、统一值模型、`fields / bind / meta / output` 这些新 DSL 核心能力当时都还没有真实实现。
- 这会让 rules DSL 路线处于“文档已经更新、代码仍然不可用”的状态，既影响维护，也会误导调用方。
- 对 `halo-spider` 来说，更合理的做法不是继续兼容旧 rules 结构，而是直接按新的 v1 DSL 重新实现 `src/rules`，并把 DSL 明确编译到共享 `Engine` 能力上。

## 范围

- 会影响哪些 capability specs：
  - `openspec/specs/rules-dsl/spec.md`
- 会影响哪些模块 / 示例：
  - `src/rules/schema.rs`
  - `src/rules/compile.rs`
  - `src/rules/run.rs`
  - `src/rules/validate.rs`
  - `src/rules/load.rs`
  - 视实现情况同步更新 `docs/guide/rules-dsl.md` 对应实现说明
- 预期带来哪些用户可见结果：
  - `Spider::rules()` 加载的新 DSL 会在 `src/rules/*` 内部按 v1 结构编译和执行。
  - `src/rules/*` 按新的 v1 单链路结构组织。
  - step 运行时支持新的 `fields / bind / follow / output / meta / allow_url_pattern` 主链路。
- DSL 中声明的 `engine` 配置会尽量在 `rules` 编译阶段直接贴合新的 request/middleware 主模型；如果 `src/runtime.rs`、`src/runtime/compile.rs` 只剩过渡职责，则在 `rules` 完成迁移后一起删除。

## 非目标

- 这次不保留历史 rules schema 的向后兼容解析；历史结构允许直接删除，不再继续维护双轨实现。
- 这次不一次性实现文档里所有潜在扩展能力；优先落地新的 v1 主骨架和最小可运行能力集。
- 这次不主动重构 `src/engine.rs`、`src/engine/task.rs` 等底层模块；但如果 `src/runtime.rs`、`src/runtime/compile.rs` 在 `rules` 迁移后只剩无用过渡层，则会一并删除并清理引用。
- 这次不重构 `store` 为另一套独立 sink runtime；DSL 的输出仍应尽量复用现有统一 item/store 主链。
- 这次不顺带扩展浏览器自动化、AI 提取或额外 middleware plugin 能力面，除非它们是让新 DSL 最小闭环跑通的必要条件。

## 风险

- 是否存在兼容性或迁移风险：
  - 存在。已有旧 rules 配置会失效，但这是刻意接受的结果；本次变更目标就是按新的 DSL v1 重写实现，而不是继续背旧模型包袱。
- 是否存在 runtime、middleware 或 plugin 相关风险：
  - 存在。rules DSL 需要尽量贴合现有底层能力；确实接不上的点应单独列成后续集成问题，而不是在当时的 change 里无边界扩到底层模块。
