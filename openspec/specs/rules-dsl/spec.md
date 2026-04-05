# 规范增量

## MODIFIED Requirements

### Requirement: DSL 围绕可执行 step 组织

库 MUST 将 DSL 中声明的运行时能力编译到共享底层链路，而不是在 rules 运行时内部发明独立能力实现。

#### Scenario: Validation compiles to shared core capability

- **WHEN** DSL step 声明 `validate`
- **THEN** 该配置编译到共享 validation 能力，而不是停留在 DSL 私有字段

#### Scenario: Validation failure stops the current DSL step explicitly

- **WHEN** DSL step 对提取字段执行共享 validation 且校验失败
- **THEN** 当前 step 返回显式 parse error，并且不继续产出 item 或 follow request

#### Scenario: Request-related DSL options compile to shared request capability

- **WHEN** DSL step 声明 request、cookies、proxy、output 等能力
- **THEN** 这些配置映射到与代码爬虫一致的底层能力模型

#### Scenario: DSL start requests apply the target step fetch on top of shared Request

- **GIVEN** spider 通过 DSL 入口启动，且起始请求命中的 step 声明了 `fetch`
- **WHEN** 引擎准备 enqueue start request
- **THEN** 引擎先构造普通共享 `Request`
- **AND** 再把目标 step 的 `fetch.request` / `fetch.browser` 显式覆盖应用到这条 `Request`

#### Scenario: DSL follow requests apply the next step fetch on top of shared follow semantics

- **GIVEN** 某个 DSL step 产出了指向下一跳 step 的 request
- **WHEN** 下一跳 step 声明了 `fetch`
- **THEN** 子请求先遵循共享 `response.follow()` 继承语义
- **AND** 再应用目标 step 的显式 request 覆盖，而不是发明 DSL 私有 request 结构

### Requirement: Parse plan 支持 fields 与 links

库 MUST 保证 `parse.fields`、`parse.links` 以及 `next_url_config` 只负责描述抓取与路由意图，而不是承载一套与代码模式脱节的私有运行时。

#### Scenario: DSL runtime does not fork core execution semantics

- **WHEN** DSL 生成 request、执行 validate、进入 runtime/middleware 或输出链路
- **THEN** 它遵循与代码爬虫相同的底层执行语义
