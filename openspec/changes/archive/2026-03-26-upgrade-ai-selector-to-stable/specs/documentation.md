# Spec: 文档更新

## 目标

更新 README 和示例，移除"实验性"标签，标记为正式功能。

## 变更内容

### README.md

1. 移除"实验性功能"标签
2. 更新为"AI 选择器"
3. 添加重试和超时配置示例
4. 更新注意事项

### 示例代码

更新 `examples/ai_extraction.rs`：
- 展示超时配置
- 展示重试配置
- 添加错误处理示例

## 新增文档内容

```rust
// 配置超时和重试
let settings = Settings::default()
    .with_openai_api_key(api_key)
    .with_openai_model("gpt-4o-mini")
    .with_ai_timeout(Duration::from_secs(30));

// 在查询中使用
let mut query = response.ai("Extract quotes")
    .with_max_retries(3)
    .with_timeout(Duration::from_secs(30));
```
