# 技术设计

## 概览

- 这次变更只收口代码爬虫公开 callback 结果模型，不重写 rules DSL 内部执行结构。
- 目标是让代码爬虫 callback 直接返回真实产物，而不是继续暴露 `Output { items, requests }`：
  - `Request`
  - `Vec<Request>`
  - `Item`
  - `Vec<Item>`
  - `()`
- 当同一个 callback 确实需要同时产出 item 和下一跳 request 时，允许直接返回 Rust tuple，例如 `(Item, Vec<Request>)` 或 `(Vec<Item>, Request)`。
- `Output` 不再作为公开 Spider API 或 `Engine::run()` 返回值存在。
- engine 内部仍然保留一层私有收口，把 callback 的直接返回值统一转换成“items + follow requests”，这样 scheduler、middleware、pipeline、store 的既有顺序不需要改写。

## 模块影响

- `src/spider.rs`
  - 删除公开 `Output` 结构。
  - 新增私有 callback 结果收口类型，例如 `CallbackOutput`。
  - 新增一个仅用于内部归一化的转换 trait，让 `Request`、`Vec<Request>`、`Item`、`Vec<Item>`、`()` 都能被统一收口。
  - `Spider::parse()` 改为公开返回“实现了内部转换 trait 的真实结果”。
  - `spider_callbacks!` / `spider_errbacks!` 继续保留，但宏内部改为把每个方法的直接返回值收口成私有输出，而不是要求用户方法显式写 `Output`。
- `src/engine/task.rs`
  - `process_spider_output(...)` 改为接收 `src/spider.rs` 的私有 callback 收口类型。
  - item pipeline、step validator、store、follow enqueue 的处理顺序保持不变。
  - callback 返回的 `Request` / `Vec<Request>` 仍然统一回到 admission 边界，不会直接绕过 scheduler。
- `src/engine.rs`
  - `Engine::run()` 不再返回 `Vec<spider::Output>`，改为只返回运行结果 `Result<(), SpiderError>`。
  - `execute_spider_once()` 与测试辅助路径改为使用内部 `TaskOutput`，不再借用公开 `spider::Output` 做测试桥接。
  - 运行结束后的统计继续通过 `engine.stats()`、signals、store 等现有边界观察。
- `examples/...`
  - 所有代码爬虫示例改成直接返回 `Request`、`Vec<Request>`、`Item` 或 `Vec<Item>`。
  - `examples/period_xml_spider.rs` 会改成：
    - `parse()` 返回列表页 `Request`
    - `parse_list()` 返回详情页 `Vec<Request>`
    - `parse_detail()` 返回最终 `Item`
  - 示例里不再出现 `Output { items, requests }`。
- `docs/...`
  - `docs/guide/getting-started.md`、`docs/capabilities.md`、相关 API 片段需要同步删掉公开 `Output` 写法。
- `openspec/specs/spider-api/spec.md`
  - 增量声明 callback 直接返回真实产物，而不是 `Output` 包装。
- `openspec/specs/runtime-engine/spec.md`
  - 增量声明 engine 会在内部收口 callback 结果，但不会把这个收口容器再暴露成公开 Spider API。

## 关键决策

- Runtime / middleware 影响：
  - 这次不改 scheduler、middleware、pipeline、store 的生命周期边界。
  - callback 结果只是从“公开 `Output` 容器”改成“公开真实产物 + 内部私有收口”。
  - callback 返回的 follow request 仍然会重新走 admission、dedup、download-before middleware 与 retry middleware，不会因为返回值模型变化而绕过既有边界。
- 对外 API 影响：
  - `Spider::parse()`、`spider_callbacks!`、`spider_errbacks!` 不再要求返回 `Result<Output, SpiderError>`。
  - 用户方法直接返回 `Result<Request, SpiderError>`、`Result<Vec<Request>, SpiderError>`、`Result<Item, SpiderError>`、`Result<Vec<Item>, SpiderError>`、`Result<(), SpiderError>`；如果同一跳要同时产出两类结果，则直接返回 Rust tuple。
  - `Engine::run()` 不再暴露 callback 输出集合；如果调用方需要读取最终 item，应使用 `store::Memory`、自定义 store、signals 或 `engine.stats()`。
  - 这是明确的破坏性变更；不提供 `Output` 兼容层。
- Plugin 或 DSL 影响：
  - plugin middleware 不受影响，因为它们只消费 request / response / item 生命周期，不感知公开 callback 返回类型。
  - rules DSL 内部的 `src/rules/run.rs::Output` 这次不改；它仍然只是 DSL 执行器内部结构，不属于代码爬虫公开 API。
  - DSL 编译和运行时边界不需要为这次变更额外增加新概念。

## 验证方式

- 编译验证：
  - `cargo fmt --all`
  - `cargo test`
- 行为验证：
  - 增加或更新 spider callback / errback 测试，覆盖：
    - `parse()` 直接返回 `Request`
    - callback 返回 `Vec<Request>`
    - callback 返回 `Item`
    - callback 返回 `Vec<Item>`
    - callback 返回 `()`
  - 验证这些结果都会被 engine 正确收口，并继续走 pipeline / validator / store / follow enqueue。
- 示例验证：
  - 更新并运行 `examples/period_xml_spider.rs`，确认链路仍然是：
    - `period.xml -> 最新版面列表 -> 详情页 -> item`
  - 示例代码里不再出现公开 `Output`。
