# 任务清单

- [x] 为 `rules::Config` 增加 local 与 inline 规则来源的易用构造函数。
- [x] 重构 `examples/quotes_dsl.rs`，改为使用 `Spider::rules()` 与引擎托管的 DSL 分发路径。
- [x] 新增或更新测试，证明 DSL-first spider 可以通过 `Engine::run()` 运行，而不需要手动接线 `compile_rules()` / `apply_dsl()`。
- [x] 更新 `README.md`，写清楚推荐的 DSL 编写流程，并说明手动 compile/apply 属于高级路径。
- [x] 运行 `cargo test`，并确认 DSL 示例至少能够成功编译。
