# Spec: 错误处理增强

## 目标

提供更清晰的错误类型和错误信息，帮助用户快速定位问题。

## 错误分类

### 1. 配置错误
- API key 未设置
- 无效的 base_url

### 2. 网络错误
- 连接超时
- DNS 解析失败
- 连接被拒绝

### 3. API 错误
- 401: API key 无效
- 429: 速率限制
- 500/502/503: 服务器错误

### 4. 响应错误
- 响应格式无效
- 响应内容为空

## 实现方式

在 `execute()` 方法中：
```rust
pub async fn execute(&mut self) -> Result<(), String> {
    let result = match self.call_api().await {
        Ok(text) => text,
        Err(e) => return Err(format_error(e)),
    };
    // ...
}
```

## 错误信息格式

- 清晰描述问题
- 提供解决建议
- 包含错误代码（如适用）
