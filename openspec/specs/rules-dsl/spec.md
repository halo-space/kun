# Rules DSL 规范

## 目标

定义 `rules()` 加载模型和 JSON DSL 的可执行结构，让规则抓取与代码抓取在一条引擎链路中共存。

### Requirement: Spider 的 rules 使用来源描述对象

库必须将 `rules()` 视为规则来源描述对象，而不是默认直接内嵌原始 DSL。

#### Scenario: local rules 从路径加载

- Given `rules().type = "local"`
- When rules loader 运行
- Then 它从 `options.path` 解析文件路径并加载 DSL 文档

#### Scenario: inline rules 可直接接收

- Given `rules().type = "inline"`
- When rules loader 运行
- Then 它把 inline DSL 文档归一化成编译后的规则格式

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

项目必须将"由框架托管的 rules 加载与分发路径"作为推荐的 DSL 编写工作流。

#### Scenario: DSL 示例不再手动编译 rules

- Given 仓库中的某个 DSL spider 示例
- When 维护者阅读或运行该示例
- Then 示例使用 `Spider::rules()` 与引擎托管的分发路径，而不是手动调用 `compile_rules()` 与 `apply_dsl()`

### Requirement: DSL 围绕可执行 step 组织

库必须将 DSL 文档建模为 `steps`，每个 step 可以声明 `id`、`callback`、`fetch`、`parse`、`route`、`output`、`runtime` 与 `MIDDLEWARES`。

#### Scenario: DSL step 可以省略 callback

- Given 某个 step 未声明 `callback`
- When 它被编译
- Then 该 step 可以省略 `callback`，转而依赖其 parse plan

#### Scenario: 代码 step 必须声明 callback

- Given 某个 step 声明了 `callback`
- When 它被编译或分发
- Then 必须存在 callback 名称

#### Scenario: step validate 编译到共享 validation plan

- Given 某个 step 声明了 `validate`
- When rules compiler 构建该 step
- Then `validate` 被编译为共享 validation plan，而不是保留为 DSL 私有原始 map

### Requirement: Fetch plan 支持 request 与 browser 细节

库必须把 step 的 fetch 配置编译成归一化的 fetch plan。

#### Scenario: HTTP fetch 编译成 HTTP request plan

- Given 某个 step 声明 `fetch.mode = "http"`
- When rules compiler 构建该 step
- Then 编译后的 fetch plan 使用 `Http` 请求模式

#### Scenario: browser fetch 保留 browser 配置

- Given 某个 step 声明 `fetch.mode = "browser"` 且包含 browser 配置
- When rules compiler 构建该 step
- Then 编译后的 fetch plan 在 request plan 旁保留 browser 配置

### Requirement: Parse plan 支持 fields 与 links

库必须在 step 中支持 `parse.fields` 与 `parse.links`。

#### Scenario: 字段提取生成结构化输出

- Given 某条字段规则声明了 `name`、`source`、`selector_type`、`selector` 与 `attribute`
- When DSL step 运行
- Then 提取出的值写入输出 item 对应字段名下

#### Scenario: 链接提取生成后续请求

- Given 某条链接规则声明了选择器与 `next_step`
- When DSL step 运行
- Then 匹配到的链接会变成带有目标 step metadata 的后续请求

#### Scenario: step validate 在运行时显式执行

- Given 某个 step 声明了 `validate`
- When DSL step 完成字段提取
- Then 引擎在产出 item 或 request 前执行共享 validation，并在失败时返回 parse error

### Requirement: DSL 解析来源与选择器类型保持显式

库必须在规则 schema 中显式保留来源类型与选择器类型。

#### Scenario: 支持的来源类型可枚举

- Given 任意字段规则或链接规则
- When 它被编译
- Then 来源从显式值中解析，例如 `html`、`text`、`json`、`xml`、`headers`、`final_url` 或 `meta.*`

#### Scenario: 支持的选择器类型可枚举

- Given 任意字段规则或链接规则
- When 它被编译
- Then 选择器类型从显式值中解析，例如 `css`、`xpath`、`json`、`xml`、`regex` 或 `ai`
