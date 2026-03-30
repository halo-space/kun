# 变更提案

## 为什么做

`halo-spider` 现在已经具备完整的规则源加载与 DSL 分发链路：

- `Spider::rules()` 可以声明规则来源
- `Engine::run()` 会自动加载并编译规则
- `Spider::dispatch()` 会根据 step 把响应路由到 DSL 或代码 callback

但当前的 [quotes_dsl 示例](/Users/xiaohan/soft/project/xiaohan/kun/examples/quotes_dsl.rs) 仍然手动读取 JSON、手动编译规则、手动在 `parse()` 里查 step 并调用 `apply_dsl()`。这会让新用户误以为 DSL 模式需要自行拼装运行链路，也让示例和正式架构产生偏差。

第一份正式变更应该优先修正这个入口体验，让“如何正确编写一个 DSL Spider”在文档、示例和 API 上保持一致。

## 范围

- 为 `rules::Config` 提供更直观的构造方式，降低 `Spider::rules()` 的使用门槛
- 重写 `examples/quotes_dsl.rs`，改为使用框架内建的规则加载与分发能力
- 补充对应的规范增量、测试和 README 说明，明确 DSL-first Spider 的推荐写法

## 非目标

- 不修改 DSL schema 本身
- 不增加新的 rules source 类型
- 不重做 Engine、Scheduler 或 Middleware 架构
- 不引入 browser anti-bot、远程规则热更新或新的插件类别

## 风险

- 示例改写后，旧的“手动 compile/apply”用法不再被推荐，可能需要在文档中明确它是底层能力而不是主路径
- 如果 `rules::Config` 的便捷构造 API 设计不清晰，可能反而引入另一套命名负担
- 需要确保纯 DSL Spider 在没有自定义 `parse()` 的情况下依然易于理解和测试
