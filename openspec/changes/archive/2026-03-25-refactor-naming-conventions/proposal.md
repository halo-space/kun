# 变更提案

## 为什么做

当前 halo-spider 的类型命名存在冗余问题：

- `HttpDownloader` 位于 `download` 模块下，类型名重复了模块语义
- `BrowserDownloader` 同样存在命名冗余
- `MemoryScheduler` 位于 `scheduler::memory` 模块下，也重复了模块名

这导致：
1. **使用体验差**：`use halo_spider::download::HttpDownloader` 显得冗长
2. **不符合 Rust 惯例**：标准库和主流 crate 通常避免这种重复（如 `std::fs::File` 而非 `std::fs::FileSystem`）
3. **维护性差**：新增类型时容易延续这种不良模式

已创建 `RUST_STYLE_GUIDE.md` 定义了命名规范，本次变更将系统性应用这些规范。

## 范围

**会影响哪些 capability specs：**
- 无需修改 specs（这是代码层面的重构，不改变功能）

**会影响哪些模块 / 示例：**
- `src/download/http.rs` - `HttpDownloader` → `Http`
- `src/download/browser.rs` - `BrowserDownloader` → `Browser`
- `src/scheduler/memory.rs` - `MemoryScheduler` → `Memory`
- `src/download.rs` - 更新导出
- `src/scheduler.rs` - 更新导出
- 所有 `examples/*.rs` - 更新导入和使用
- `README.md` - 更新示例代码

**预期带来哪些用户可见结果：**
- 更简洁的 API：`use halo_spider::download::Http` 替代 `use halo_spider::download::HttpDownloader`
- 更符合 Rust 生态惯例的命名风格
- 更好的开发体验

## 非目标

- **不改变功能**：这是纯重构，不添加新功能
- **不修改内部逻辑**：只改类型名，不改实现
- **不涉及其他命名**：仅重构这三个明显冗余的类型，其他类型（如 `Settings`、`Engine`）保持不变

## 风险

**兼容性风险：**
- 这是破坏性变更，会影响所有使用这些类型的用户代码
- 缓解措施：当前版本是 0.0.4，在 1.0 之前可以接受破坏性变更

**迁移风险：**
- 用户需要更新导入语句
- 缓解措施：变更简单直接，IDE 的查找替换即可完成迁移
