# Spider API 增量规范

## ADDED Requirements

### Requirement: DSL-first spider 可以在没有自定义分发胶水代码的情况下声明 rules

库必须支持这类 spider：它们依赖 `rules()` 与引擎托管的分发路径，而不需要在 `parse()` 内手动加载、编译或执行 DSL rules。

#### Scenario: 纯 DSL spider 使用默认 parse 路由

- Given spider 提供了 `start_urls()` 与 `rules()`
- And 编译后的 rules 中存在 `id = "parse"` 的 DSL step
- When `Engine::run()` 执行该 spider
- Then 响应由 DSL step 处理，不需要自定义 `parse()` 胶水代码

#### Scenario: DSL spider 仍可与代码回调共存

- Given spider 提供了 `rules()`
- And 部分 step 使用 `impl = "dsl"`，另一些 step 使用 `impl = "code"`
- When 引擎分发响应
- Then DSL step 走 rules 引擎，代码 step 走同一 spider 上的具名回调
