# 任务清单

## 1. Spider 公开 callback 返回模型

- [x] 1.1 删除 `src/spider.rs` 中公开的 `Output` 结构，并把 `Spider::parse()`、`spider_callbacks!`、`spider_errbacks!` 改成直接接收 `Request`、`Vec<Request>`、`Item`、`Vec<Item>`、`()` 这类真实返回值。
- [x] 1.2 在 `src/spider.rs` 中补一层私有 callback 结果收口，让 engine 仍然可以统一处理 items 和 follow requests，但这层结构不再作为公开 API 暴露。

## 2. Engine 内部接线

- [x] 2.1 更新 `src/engine/task.rs`，让 callback / errback 的直接返回值继续按既有顺序进入 pipeline、validator、store 与 follow enqueue。
- [x] 2.2 更新 `src/engine.rs`，移除 `Engine::run()` 与 `execute_spider_once()` 对公开 `spider::Output` 的依赖，并把 `Engine::run()` 的公开返回值收口为纯运行结果。

## 3. 示例、测试与文档

- [x] 3.1 更新 `examples/period_xml_spider.rs` 及其它代码爬虫示例，全部删除公开 `Output { items, requests }` 写法。
- [x] 3.2 更新 `src/engine.rs`、`src/spider.rs` 里的相关测试，覆盖 callback / errback 直接返回 `Request`、`Vec<Request>`、`Item`、`Vec<Item>`、`()` 的场景。
- [x] 3.3 同步 README 与 docs 里的示例片段，确保公开文档不再引用 `Output::empty()` 或 `Result<Output, SpiderError>`。

## 4. 验证

- [x] 4.1 运行 `cargo fmt --all`。
- [x] 4.2 运行 `cargo test` 或至少覆盖 spider / engine / examples 相关定向测试。
- [x] 4.3 运行 `cargo run --example period_xml_spider --quiet`，验证 `period.xml -> 列表页 -> 详情页 -> item` 示例链路仍然可用。
