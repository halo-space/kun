# 规范增量

## ADDED Requirements

### Requirement: Engine Normalizes Direct Spider Callback Results Internally

系统 MUST 在 engine 内部统一收口 spider callback 的直接返回值，但这个收口结构不能继续作为公开 Spider API 暴露。

#### Scenario: Engine normalizes request results into follow work

- **WHEN** spider callback 直接返回 `Request` 或 `Vec<Request>`
- **THEN** engine 会在内部把它们收口成 follow work
- **AND** 这些 request 会继续重新进入 admission 与 scheduler 边界

#### Scenario: Engine normalizes item results into item pipeline input

- **WHEN** spider callback 直接返回 `Item` 或 `Vec<Item>`
- **THEN** engine 会在内部把它们收口成 item 集合
- **AND** 它们继续进入现有 pipeline、validator 与 store 链路

#### Scenario: Internal normalization is not a public output API

- **WHEN** engine 需要统一处理 callback 返回值
- **THEN** 它可以使用私有内部结构收口
- **AND** 调用方不再通过公开 `spider::Output` 感知这层内部结构

#### Scenario: Tuple callback results normalize into both items and follow requests

- **WHEN** spider callback 直接返回 Rust tuple，例如 `(Item, Vec<Request>)`
- **THEN** engine 会把 tuple 左侧收口成 item 集合，把右侧收口成 follow request 集合
- **AND** 这条路径继续走同一条 pipeline、validator、store 与 admission 主链

### Requirement: Engine Run Completion Does Not Expose Callback Output Containers

系统 MUST 不再把 callback 输出容器作为 `Engine::run()` 的公开返回值暴露。

#### Scenario: Engine run returns completion status only

- **WHEN** 调用方执行 `Engine::run(&spider).await`
- **THEN** 公开返回值只表达运行是否成功完成
- **AND** 不再返回 `Vec<spider::Output>` 一类 callback 输出集合

#### Scenario: Final item observation stays on store stats and signals boundaries

- **WHEN** 调用方需要观察最终抓到的 item
- **THEN** 它通过 store、signals 或 stats 这类既有运行时边界观察结果
- **AND** engine 不再把 callback 中间产物包装成单独的公开结果集合

## MODIFIED Requirements

### Requirement: Engine Processes Spider Callback Outputs As Request-Scoped Work

系统 MUST 把 spider callback 返回的 request / item 收口回 engine 的固定执行边界，而不是让调用方自己推断后续执行顺序。

#### Scenario: Callback output requests re-enter admission after callback returns

- **WHEN** spider callback 直接返回了一条新的 `Request` 或 `Vec<Request>`
- **THEN** engine 在 callback 返回后统一接管这些 request
- **AND** 它们会重新进入 admission 边界，再按自己的 effective request runtime 执行 dedup / download-before middleware / retry middleware

#### Scenario: Callback output handling does not bypass runtime boundaries

- **WHEN** spider callback 直接返回 `Request`、`Vec<Request>`、`Item`、`Vec<Item>` 或 `()`
- **THEN** 这些输出只表达“产出下一批工作”
- **AND** 它们不会绕过 scheduler、admission、download attempt 或 store/pipeline 这些既定 engine 边界
