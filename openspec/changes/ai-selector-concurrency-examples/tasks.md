# 任务清单

## 1. 依赖配置

- [x] 1.1 在 `Cargo.toml` 中添加 `async-openai = "0.34"` 依赖，作为可选 feature `ai-selector`
- [x] 1.2 更新 `Cargo.toml` 的 features 部分，添加 `ai-selector = ["dep:async-openai"]`

## 2. AI 选择器实现

- [x] 2.1 在 `src/parser/ai.rs` 中实现 OpenAI API 调用逻辑
- [x] 2.2 修改 `src/rules/run.rs`，将 `apply` 和相关函数改为 async
- [x] 2.3 更新调用 `apply` 的地方以支持 async

## 3. 并发控制优化

- [x] 3.1 在 `src/settings.rs` 中添加 `connection_pool_size` 字段和构造方法
- [x] 3.2 在 `src/settings.rs` 中添加 `openai_api_key`、`openai_base_url` 和 `openai_model` 字段
- [x] 3.3 修改 `src/engine.rs`，实现按域名的并发控制（已存在实现）
- [x] 3.4 在 HTTP 客户端创建时应用 `connection_pool_size` 配置（通过 `HttpDownloader::with_pool_size()`）

## 4. 示例代码（核心）

- [x] 4.1 创建 `examples/ai_extraction.rs`，演示 AI 选择器用法
- [x] 4.2 创建 `examples/concurrency_control.rs`，演示并发配置

## 4+. 示例代码（TODO - 后续补充）

- [ ] 4.3 创建 `examples/ecommerce.rs`，电商爬取场景
- [ ] 4.4 创建 `examples/news_aggregator.rs`，新闻聚合场景
- [ ] 4.5 创建 `examples/api_spider.rs`，API 数据抓取场景
- [ ] 4.6 创建 `examples/browser_automation.rs`，浏览器模式完整示例

## 5. 文档更新

- [x] 5.1 更新 `README.md`，添加 AI 选择器使用说明和成本提示
- [x] 5.2 更新 `README.md`，添加并发控制配置说明
- [x] 5.3 在 `README.md` 中添加新示例的索引

## 6. API 统一化（额外任务）

- [x] 6.1 统一 Settings 所有 builder 方法为 `with_` 前缀
- [x] 6.2 更新所有示例文件使用新 API
- [x] 6.3 更新 README 文档中的示例代码

## 7. 验证

- [x] 7.1 运行 `cargo test` 确保所有测试通过 (86 passed)
- [x] 7.2 运行 `cargo check --features ai-selector` 确保 AI 功能编译通过
- [ ] 7.3 运行至少一个示例验证功能：`cargo run --example ai_extraction --features ai-selector`
- [x] 7.4 运行 `cargo clippy` 确保代码质量 (仅有风格警告)
