# 变更提案

## 为什么做

- 当前代码爬虫 callback 统一返回 `Result<spider::Output, SpiderError>`，把“继续调度 request”和“产出最终 item”硬塞进同一个 `Output { items, requests }` 容器。
- 这个模型会让代码爬虫的实际语义变得很重：列表页明明只是产出下一跳 request，详情页明明只是产出 item，但用户仍然必须围绕 `Output` 组装 `items` / `requests` 两个字段。
- 这种设计已经开始反向影响示例写法与 API 心智。像 `examples/period_xml_spider.rs` 这样的链路，本应直观表达成“列表页返回 request，详情页返回 item”，现在却只能继续强化 `Output` 这个混合容器。
- 对 `halo-spider` 的代码爬虫用户来说，这会抬高上手成本，也会让 `Request::new(url).with_callback(...).with_meta_map(...)` 这一类更自然的写法被外层 `Output` 包装稀释掉。

## 范围

- 会影响哪些 capability specs：
  - `openspec/specs/spider-api/spec.md`
  - `openspec/specs/runtime-engine/spec.md`
- 会影响哪些模块 / 示例：
  - `src/spider.rs`
  - `src/engine.rs`
  - `src/engine/task.rs`
  - 依赖 `Spider` callback 返回值的示例与测试
- 预期带来哪些用户可见结果：
  - 代码爬虫 callback 不再显式返回 `Output`
  - callback 直接返回真实产物：`Request`、`Vec<Request>`、`Item`、`Vec<Item>`
  - `spider_callbacks!` / `spider_errbacks!` 继续可用，但用户方法不再被迫围绕 `Output` 写样板代码
  - 示例会改成更直接的 request / item 返回风格

## 非目标

- 这次不同时重写 rules DSL 运行时内部的 `src/rules/run.rs::Output` 结构；该结构属于 rules 内部执行收口，不等同于代码爬虫公开 API。
- 这次不引入新的发射器、事件总线或 `emit(...)` 一类额外概念，只解决 callback 返回值模型本身过重的问题。
- 这次不顺手改 request builder 为伪异步接口；`Request::new(...).with_xxx(...)` 仍保持同步 builder。

## 风险

- 是否存在兼容性或迁移风险：
  - 存在。所有直接实现 `Spider::parse()`、`Spider::call()`、`Spider::handle_error()` 并显式返回 `Output` 的代码与示例都需要迁移。
  - `spider_callbacks!` / `spider_errbacks!` 宏的生成行为会变化，需要同步更新文档和示例，避免用户继续照旧写 `Output`。
- 是否存在 runtime、middleware 或 plugin 相关风险：
  - engine 当前在 `TaskExecutor::process_spider_output(...)` 之后统一处理 pipeline、validator、store 与 follow enqueue，因此内部仍需要一个统一收口层；若收口设计不清楚，可能影响 errback、调度回写与测试稳定性。
  - 需要确保“callback 直接返回 request / item”后，现有 middleware、pipeline、scheduler resolve 与 signals 语义保持不变，只改变公开 Spider API 的写法与收口方式。
