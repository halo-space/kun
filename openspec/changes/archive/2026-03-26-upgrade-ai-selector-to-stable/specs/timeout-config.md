# Spec: 超时配置

## 目标

为 AI 请求添加超时配置，避免长时间等待。

## 接口变更

### AiQuery 结构体

新增字段：
```rust
pub timeout: Duration,  // 默认 30 秒
```

### 配置方法

```rust
impl AiQuery {
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}
```

## Settings 集成

```rust
impl Settings {
    pub fn with_ai_timeout(mut self, timeout: Duration) -> Self {
        self.ai_timeout = Some(timeout);
        self
    }
}
```

## 实现要点

- 使用 `tokio::time::timeout` 包装 API 调用
- 超时返回明确的错误信息
- 超时视为可重试错误
