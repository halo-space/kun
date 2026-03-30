# 任务清单

## 1. 重命名 HttpDownloader → Http

- [x] 1.1 在 `src/download/http.rs` 中重命名 `pub struct HttpDownloader` 为 `Http`
- [x] 1.2 在 `src/download/http.rs` 中重命名所有 `impl HttpDownloader` 为 `impl Http`
- [x] 1.3 在 `src/download.rs` 中更新 `pub use http::HttpDownloader` 为 `pub use http::Http`

## 2. 重命名 BrowserDownloader → Browser

- [x] 2.1 在 `src/download/browser.rs` 中重命名 `pub struct BrowserDownloader` 为 `Browser`
- [x] 2.2 在 `src/download/browser.rs` 中重命名所有 `impl BrowserDownloader` 为 `impl Browser`
- [x] 2.3 在 `src/download.rs` 中更新 `pub use browser::BrowserDownloader` 为 `pub use browser::Browser`

## 3. 重命名 MemoryScheduler → Memory

- [x] 3.1 在 `src/scheduler/memory.rs` 中重命名 `pub struct MemoryScheduler` 为 `Memory`
- [x] 3.2 在 `src/scheduler/memory.rs` 中重命名所有 `impl MemoryScheduler` 为 `impl Memory`
- [x] 3.3 在 `src/scheduler.rs` 中更新 `pub use memory::MemoryScheduler` 为 `pub use memory::Memory`

## 4. 更新 src/ 中的引用

- [x] 4.1 在 `src/engine.rs` 中更新所有类型引用（主要在测试代码中）
- [x] 4.2 检查其他 src/ 文件中是否有引用需要更新

## 5. 更新示例文件

- [x] 5.1 更新 `examples/quotes_code.rs`
- [x] 5.2 更新 `examples/quotes_dsl.rs`
- [x] 5.3 更新 `examples/custom_middleware.rs`
- [x] 5.4 更新 `examples/plugins_demo.rs`
- [x] 5.5 更新 `examples/ai_extraction.rs`
- [x] 5.6 更新 `examples/concurrency_control.rs`

## 6. 更新文档

- [x] 6.1 更新 `README.md` 中的快速开始示例
- [x] 6.2 更新 `README.md` 中的其他示例代码

## 7. 验证

- [x] 7.1 运行 `cargo check` 确保编译通过
- [x] 7.2 运行 `cargo test` 确保所有测试通过
- [x] 7.3 运行 `cargo clippy` 确保无新增警告
- [x] 7.4 运行至少一个示例验证可执行性
