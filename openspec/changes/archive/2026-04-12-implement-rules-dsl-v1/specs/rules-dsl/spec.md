# 规范增量

## MODIFIED Requirements

### Requirement: Rules DSL 使用新的 v1 authored 结构

库 MUST 在 `src/rules/*` 内以新的 `spider / engine / sinks / seeds / steps` authored schema 组织 rules。

#### Scenario: Compiler parses the new top-level DSL structure

- **WHEN** spider 通过 `Spider::rules()` 提供新的 DSL v1 配置
- **THEN** rules compiler 解析 `spider`、`engine`、`sinks`、`seeds` 与 `steps` 这些顶层结构
- **AND** 编译结果围绕新的单链路 plan 组织

#### Scenario: Non-v1 rules structures are no longer supported

- **WHEN** 调用方继续提交非 v1 authored 结构
- **THEN** 这些字段不再作为 v1 rules DSL 的兼容输入继续支持
- **AND** 调用方需要迁移到新的 `fields / bind / follow / output` 单链路模型

### Requirement: Step 以单条链路执行 fields bind follow output

库 MUST 让每个 step 围绕“当前请求 -> 解析 -> 生成下一跳或输出 item”这条单链路执行，而不是引入额外的批处理语法。

#### Scenario: Step execution follows fields bind follow output order

- **WHEN** 某个 DSL step 被执行
- **THEN** 运行时先解析 `fields`
- **AND** 再计算 `bind`
- **AND** 最后生成 `follow` request 或 `output` item

#### Scenario: Follow item scope produces one child request per matched node

- **WHEN** `follow` 声明了 `item`
- **THEN** 运行时在该节点作用域内分别计算 `request` 与 `meta`
- **AND** 每个命中的节点生成一条独立子链路
- **AND** 该子链路会把当前 URL 对应的 `meta` 与请求绑定后继续流转

#### Scenario: Output validate uses the shared validation capability

- **WHEN** step 声明了 `output.item` 与 `output.validate`
- **THEN** 运行时组装统一 item 输出
- **AND** `output.validate` 映射到共享 validation 能力
- **AND** 校验通过的 item 继续走统一的 engine `pipeline -> store` 主链

### Requirement: DSL engine 配置贴合现有共享底层能力

库 MUST 让 DSL v1 中的 `engine` 配置优先贴合当前已有的底层能力。

#### Scenario: Top-level engine registries can be referenced inside rules

- **WHEN** 顶层声明了 `engine.dedup`、`engine.concurrency`、`engine.interval`、`engine.rate_limit`、`engine.auto_throttle`、`engine.retry_by_status` 或 `engine.retry_by_error`
- **THEN** seed 或 follow 可以通过命名引用使用这些规则
- **AND** 编译阶段会校验这些引用确实存在

#### Scenario: Rules compiler lowers engine config to the current middleware model

- **WHEN** 某个 DSL `engine` 配置可以在 `src/rules/*` 内直接映射到现有底层能力
- **THEN** compiler 在 `rules` 边界内完成 lowering
- **AND** 不要求先修改底层 runtime/engine 代码

### Requirement: Seeds 与 sinks 接入当前统一主链

库 MUST 在新的 authored schema 与 compiled plan 中保留 `seeds`、`sinks` 这些 DSL 配置面，并把它们接入当前统一主链。

#### Scenario: Seeds drive engine start requests

- **WHEN** DSL 顶层声明了 `seeds`
- **THEN** 引擎会把这些 seed 解析成实际起始请求
- **AND** 当 `rules.seeds` 为空时，仍回退到 `Spider::build_start_requests()` / `start_urls()`

#### Scenario: Sinks route output through the unified store path

- **WHEN** DSL 顶层声明了 `sinks`，且 step `output` 引用了其中的 sink
- **THEN** rules compiler 会校验这些引用关系
- **AND** output 继续复用统一 `item -> pipeline -> store` 主链
- **AND** `output.sinks` 会被解析为目标 store 路由，而不是引入第二套独立 sink runtime
