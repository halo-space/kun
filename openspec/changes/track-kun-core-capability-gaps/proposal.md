# 变更提案

## 为什么做

- 当前 `halo-spider` 已经把 `dedup / retry / schedule` 这类运行时能力逐步收敛到了共享底层链路，但仍有一批核心能力处于“不完整”“只有 DSL 配置入口”“只有壳没有真正实现”或“行为边界不清晰”的状态。
- 如果不把这些缺口先做成正式的 OpenSpec change 与任务清单，后续继续推进 DSL、示例或模块扩展时，很容易重复讨论、遗漏未实现项，或者把 DSL 配置误当成底层能力本身。
- 这次 change 的目标不是一次性实现所有能力，而是把 kun 当前底层能力缺口整理成可追踪、可分阶段实现的正式计划，作为后续逐项补齐的执行入口。

## 范围

- 会影响哪些 capability specs：
  - `openspec/specs/spider-api/spec.md`
  - `openspec/specs/runtime-engine/spec.md`
  - `openspec/specs/middleware-plugins/spec.md`
  - `openspec/specs/rules-dsl/spec.md`
- 会影响哪些模块 / 示例：
  - `src/request.rs`
  - `src/response.rs`
  - `src/response/follow.rs`
  - `src/engine.rs`
  - `src/scheduler/*`
  - `src/middleware/*`
  - `src/download/*`
  - `src/parser/*`
  - `src/pipeline.rs`
  - `src/store.rs`
  - `src/plugins/*`
  - `src/rules/*`
- 预期带来哪些用户可见结果：
  - 明确哪些能力已经属于共享底层能力，哪些还只是 DSL 字段或占位实现
  - 为后续按优先级补齐 `validate`、request/follow、scheduler identity、cookies/proxy、browser、parser、store/plugin 等能力提供正式任务清单
  - 保证后续实现工作以 `src/` 的真实底层能力为核心，而不是继续围绕 DSL 表层配置扩散

## 非目标

- 这次 change 不重做 DSL 配置面设计。
- 这次 change 不优先围绕 `rules` / DSL 表层配置扩展能力。
- 这次 change 不把现有架构整体推翻重写，而是在当前 `Spider / Request / Engine / Response / runtime / middleware` 结构上逐步补齐。

## 风险

- 是否存在兼容性或迁移风险：
  - 存在。后续在补齐 request/follow、scheduler identity、browser 行为和 validate 共享能力时，可能需要收紧部分当前较宽松或占位的行为。
- 是否存在 runtime、middleware 或 plugin 相关风险：
  - 存在。`cookies`、`proxy`、`store`、`plugin kind` 等能力如果直接补实现，容易与当前 middleware/plugin 边界重新耦合，因此需要分阶段推进并逐步验证。
