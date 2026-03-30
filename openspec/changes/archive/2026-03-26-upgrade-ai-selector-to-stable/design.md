# Design: 将 AI 选择器升级为正式功能

## 架构设计

### 1. 重试机制

使用指数退避策略：
- 默认最大重试次数：3 次
- 退避时间：1s, 2s, 4s
- 可配置的重试次数
- 仅对可重试错误（网络超时、5xx）进行重试

### 2. 错误处理

增强错误类型：
- `ApiKeyMissing`: API key 未配置
- `NetworkError`: 网络连接失败
- `Timeout`: 请求超时
- `RateLimitExceeded`: 超过速率限制
- `InvalidResponse`: 响应格式错误

### 3. 超时配置

- 默认超时：30 秒
- 可通过 Settings 配置
- 单独的连接超时和读取超时

## 数据结构变更

### AiQuery 结构体

```rust
pub struct AiQuery {
    // 现有字段
    pub input: String,
    pub prompt: String,
    pub source: Option<String>,
    pub value: ValueQuery,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: String,

    // 新增字段
    pub max_retries: u32,
    pub timeout: Duration,
}
```

## 实现细节

### 重试逻辑

```rust
async fn execute_with_retry() -> Result<String, String> {
    let mut attempt = 0;
    loop {
        match openai_call().await {
            Ok(result) => return Ok(result),
            Err(e) if is_retryable(&e) && attempt < max_retries => {
                attempt += 1;
                sleep(Duration::from_secs(2u64.pow(attempt))).await;
            }
            Err(e) => return Err(e),
        }
    }
}
```

### 向后兼容

- 保持现有 API 不变
- 新增字段使用默认值
- 不破坏现有用户代码

## 测试策略

- 单元测试：重试逻辑、错误处理
- 集成测试：完整的 AI 查询流程
- Mock OpenAI API 进行测试
