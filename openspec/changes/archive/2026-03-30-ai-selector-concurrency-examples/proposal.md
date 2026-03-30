# 变更提案

## 为什么做

当前 halo-spider 存在以下问题：

1. **AI 选择器未实现**：rules-dsl spec 中已定义 `ai` 选择器类型，但代码中尚未实现，用户无法使用 AI 进行智能内容提取
2. **并发控制不完善**：虽然 runtime-engine spec 提到全局并发控制，但缺少连接池优化、按域名并发限制的具体实现
3. **示例不足**：examples/ 目录缺少展示不同使用场景的示例，新用户难以快速上手

这些问题影响了框架的易用性和生产环境适用性。

## 范围

**会影响哪些 capability specs：**
- `rules-dsl`：新增 AI 选择器实现规范
- `runtime-engine`：完善并发控制与连接池优化规范
- `spider-api`：可能需要扩展 Spider trait 以支持 AI 配置

**会影响哪些模块 / 示例：**
- `src/rules/`：实现 AI 选择器解析与执行
- `src/engine.rs` 或 `src/settings.rs`：增强并发控制与连接池配置
- `Cargo.toml`：添加 `async-openai` 依赖
- `examples/`：新增多个场景示例（电商、新闻、API、浏览器模式等）

**预期带来哪些用户可见结果：**
- 用户可在 DSL 中使用 `selector_type: "ai"` 进行智能提取
- 用户可配置更精细的并发控制（全局、按域名、连接池大小）
- 用户可参考丰富的示例快速实现各类爬虫场景

## 非目标

- **不涉及其他 AI 提供商**：本次仅集成 OpenAI，其他提供商（Claude、Gemini 等）留待后续扩展
- **不重构现有中间件架构**：并发优化在现有 Settings/Engine 框架内实现
- **不改变 DSL 核心结构**：AI 选择器作为新的 selector_type 添加，不改变现有 parse plan 结构

## 风险

**兼容性风险：**
- AI 选择器需要 API key 配置，需要设计合理的配置传递机制（环境变量 vs Settings）
- 新增依赖 `async-openai` 可能增加编译时间和二进制大小

**Runtime 风险：**
- AI 调用有网络延迟和费用，需要在文档中明确说明使用场景和成本考虑
- 并发控制优化可能影响现有 middleware 行为，需要充分测试
