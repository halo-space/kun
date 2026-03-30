# Rules DSL 增量规范

## ADDED Requirements

### Requirement: rules 配置支持易用的来源构造函数

库必须为公开的 rules 来源描述对象提供更易用的构造函数，让编写者无需手动拼装底层 map，就能声明 local 与 inline 规则来源。

#### Scenario: local rules 配置可由路径构造

- Given 用户希望从 JSON 文件加载 rules
- When 用户调用 rules 配置类型上的 local 构造函数
- Then 生成的配置使用 `local` 作为来源类型，并把文件路径写入预期的 options 字段

#### Scenario: inline rules 配置可由值构造

- Given 用户希望直接提供 inline rules
- When 用户调用 rules 配置类型上的 inline 构造函数
- Then 生成的配置使用 `inline` 作为来源类型，并把 inline rules 文档写入预期的 options 字段

### Requirement: 官方 DSL 示例遵循引擎托管的 rules 流程

项目必须将“由框架托管的 rules 加载与分发路径”作为推荐的 DSL 编写工作流。

#### Scenario: DSL 示例不再手动编译 rules

- Given 仓库中的某个 DSL spider 示例
- When 维护者阅读或运行该示例
- Then 示例使用 `Spider::rules()` 与引擎托管的分发路径，而不是手动调用 `compile_rules()` 与 `apply_dsl()`
