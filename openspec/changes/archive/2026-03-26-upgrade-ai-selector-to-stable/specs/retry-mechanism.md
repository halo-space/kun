# Spec: AI Query 重试机制

## 目标

为 AiQuery 添加自动重试机制，提高网络不稳定情况下的成功率。

## 接口变更

### AiQuery 结构体

新增字段：
```rust
pub max_retries: u32,  // 默认 3
```

### 配置方法

```rust
impl AiQuery {
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }
}
```

## 重试策略

- 指数退避：1s, 2s, 4s
- 仅重试可恢复错误：网络超时、5xx 错误
- 不重试：4xx 错误、API key 错误

## 实现要点

- 在 `execute()` 方法中实现重试循环
- 使用 `tokio::time::sleep` 实现延迟
- 记录重试日志（使用 tracing）
