# 技术设计

## 概览

本次变更分为三个主要部分：
1. **AI 选择器实现**：在 rules DSL 中支持 `selector_type: "ai"`，使用 `async-openai` 调用 OpenAI API 进行智能内容提取
2. **并发控制优化**：完善现有的并发控制机制，增加连接池配置和更精细的域名级并发控制
3. **示例扩充**：添加多个实际场景的示例代码

## 模块影响

### src/ 模块变更

**新增模块：**
- `src/selector/ai.rs`：AI 选择器实现，封装 OpenAI API 调用
- `src/selector/mod.rs`：选择器模块入口，统一管理 CSS/XPath/Regex/AI 选择器

**修改模块：**
- `src/rules/run.rs`：在 DSL 执行时支持 AI 选择器类型
- `src/settings.rs`：添加 OpenAI API key 配置和连接池配置
- `src/engine.rs`：优化并发控制逻辑，实现按域名的并发限制
- `src/lib.rs`：导出新增的 selector 模块

**依赖变更：**
- `Cargo.toml`：添加 `async-openai = "0.24"` 依赖（可选 feature）

### examples/ 变更

新增示例：
- `examples/ai_extraction.rs`：演示 AI 选择器提取复杂内容
- `examples/ecommerce.rs`：电商网站爬取示例
- `examples/news_aggregator.rs`：新闻聚合示例
- `examples/api_spider.rs`：API 数据抓取示例
- `examples/browser_automation.rs`：浏览器模式完整示例
- `examples/concurrency_control.rs`：并发控制配置示例

### openspec/specs/ 变更

需要更新的 spec：
- `openspec/specs/rules-dsl/spec.md`：补充 AI 选择器的详细规范
- `openspec/specs/runtime-engine/spec.md`：补充连接池和域名并发控制规范

## 关键决策

### Runtime / middleware 影响

**并发控制实现方式：**
- 在 `Engine` 中使用 `Semaphore` 实现全局并发限制（已有）
- 新增 `HashMap<Domain, Semaphore>` 实现按域名并发限制
- 在 `Settings` 中添加 `connection_pool_size` 配置，传递给 `reqwest::Client`

**不改变现有 middleware 链：**
- 并发控制在 Engine 层面实现，不作为 middleware
- 保持现有 retry、dedup、rate_limit 等 middleware 的独立性

### 对外 API 影响

**Settings 新增配置项：**
```rust
pub struct Settings {
    // 现有字段...
    pub openai_api_key: Option<String>,
    pub openai_model: String,
    pub connection_pool_size: usize,
}
```

**新增 feature flag：**
```toml
[features]
ai-selector = ["dep:async-openai"]
```

用户需要显式启用 `ai-selector` feature 才能使用 AI 选择器。

### Plugin 或 DSL 影响

**AI 选择器 DSL 用法：**
```json
{
  "name": "summary",
  "source": "html",
  "selector_type": "ai",
  "selector": ["Extract the main article summary in 2-3 sentences"],
  "attribute": "text"
}
```

**配置传递：**
- API key 优先从环境变量 `OPENAI_API_KEY` 读取
- 也可通过 `Settings::openai_api_key()` 显式配置
- 模型默认使用 `gpt-4o-mini`，可通过 `Settings::openai_model()` 配置

## 验证方式

1. **单元测试**：
   - `src/selector/ai.rs` 的 mock 测试
   - 并发控制逻辑的测试

2. **集成测试**：
   - 运行所有新增示例：`cargo run --example <name> --features ai-selector`
   - 验证并发限制是否生效

3. **文档验证**：
   - 更新 `README.md`，说明 AI 选择器的使用方法和成本考虑
   - 确保所有示例都有清晰的注释和说明

4. **性能测试**：
   - 验证连接池优化对性能的影响
   - 测试按域名并发限制是否正常工作
