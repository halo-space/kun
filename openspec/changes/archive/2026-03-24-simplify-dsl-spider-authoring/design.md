# 技术设计

## 概览

这个 change 的目标不是扩展新能力，而是把已经存在的 DSL 执行模型收拢成更易用、更一致的作者体验。

目标状态：

- 纯 DSL Spider 可以只声明 `name`、`start_urls()` 和 `rules()`
- 示例不再手动读取规则文件、不再手动 `compile_rules()`、不再手动 `apply_dsl()`
- `rules::Config` 提供面向作者的便捷构造方法，避免用户每次手写 `type` 和 `options`

## 模块影响

- `src/rules/schema.rs`
  - 为 `Config` 增加构造辅助方法，例如 `local(path)` 和 `inline(value)`
- `examples/quotes_dsl.rs`
  - 改写为通过 `Spider::rules()` 声明 `examples/rules/quotes.json`
  - 去掉手动编译和手动 DSL 分发逻辑
- `README.md`
  - 把 DSL Spider 的推荐写法同步成框架内建 workflow
- `openspec/specs/`
  - 暂不直接修改主 specs，本 change 通过 delta specs 先定义预期行为

## 关键决策

### 以引擎托管的 DSL 路径作为推荐流程

当前引擎已经具备这些行为：

- `Engine::run()` 自动加载 `Spider::rules()`
- `Spider::dispatch()` 默认根据 `next_step` 或 `"parse"` 路由
- `impl = "dsl"` 的 step 会直接走 DSL 执行

因此推荐路径应该显式建立在现有能力上，而不是继续展示手动 glue code。

### 增加易用构造函数，而不是引入新的 helper trait

为了避免增加新的概念层，优先在现有 `rules::Config` 上增加构造函数：

- `Config::local(path)`
- `Config::inline(value)`

这样用户仍然面对同一个公开类型，只是写法更短、更不易出错。

### 保留手动 compile/apply 作为底层逃生口

`compile_rules()` 与 `apply_dsl()` 仍然保留，用于高级场景、测试或内部控制。但示例和 README 不再把它们作为 DSL Spider 的默认入门路径。

## 验证方式

- 为 `Config` 便捷构造方法添加单元测试
- 添加或更新引擎级测试，覆盖 `Spider::rules()` + 默认 `"parse"` step 的 DSL 路径
- 运行 `cargo test`
- 运行或至少编译 `examples/quotes_dsl.rs`
