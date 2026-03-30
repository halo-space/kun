# 技术设计

## 概览

本次重构采用全局查找替换的方式，系统性地将三个类型重命名：
- `HttpDownloader` → `Http`
- `BrowserDownloader` → `Browser`
- `MemoryScheduler` → `Memory`

这是纯代码层面的重构，不涉及功能变更或架构调整。

## 模块影响

### src/ 模块变更

**定义文件：**
- `src/download/http.rs` - 重命名 `pub struct HttpDownloader` 及其所有 impl 块
- `src/download/browser.rs` - 重命名 `pub struct BrowserDownloader` 及其所有 impl 块
- `src/scheduler/memory.rs` - 重命名 `pub struct MemoryScheduler` 及其所有 impl 块

**导出文件：**
- `src/download.rs` - 更新 `pub use` 语句
- `src/scheduler.rs` - 更新 `pub use` 语句

**使用位置：**
- `src/engine.rs` - 测试代码中的类型引用
- 其他可能引用这些类型的模块

### examples/ 变更

所有示例文件需要更新：
- `examples/quotes_code.rs`
- `examples/quotes_dsl.rs`
- `examples/custom_middleware.rs`
- `examples/plugins_demo.rs`
- `examples/ai_extraction.rs`
- `examples/concurrency_control.rs`

### 文档变更

- `README.md` - 更新快速开始和示例代码中的类型引用

## 关键决策

### Runtime / middleware 影响

无影响。这些类型仅作为 Engine 的泛型参数，重命名不改变运行时行为。

### 对外 API 影响

**破坏性变更：**
- 所有使用 `HttpDownloader`、`BrowserDownloader`、`MemoryScheduler` 的用户代码需要更新
- 由于当前版本是 0.0.4（pre-1.0），可以接受破坏性变更

**迁移路径：**
```rust
// 旧代码
use halo_spider::download::{HttpDownloader, BrowserDownloader};
use halo_spider::scheduler::memory::MemoryScheduler;

// 新代码
use halo_spider::download::{Http, Browser};
use halo_spider::scheduler::memory::Memory;
```

### Plugin 或 DSL 影响

无影响。DSL 和 plugin 系统不直接引用这些类型。

## 验证方式

1. **编译验证**：`cargo check` 和 `cargo build` 确保所有代码编译通过
2. **测试验证**：`cargo test` 确保所有测试通过
3. **示例验证**：运行所有示例确保可执行
4. **代码质量**：`cargo clippy` 确保无新增警告
