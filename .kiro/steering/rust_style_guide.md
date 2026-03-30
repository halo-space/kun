# Rust 代码规范

## 命名约定

### 模块和类型命名

**原则：避免在类型名中重复模块名**

当类型位于特定功能模块下时，类型名应简洁，不需要重复模块的语义。

#### ✅ 推荐

```rust
// 模块：download
pub struct Http { }
pub struct Browser { }

// 使用
use crate::download::{Http, Browser};
let http = Http::new();
let browser = Browser::new();
```

#### ❌ 不推荐

```rust
// 模块：download
pub struct HttpDownloader { }      // 重复了 "download" 语义
pub struct BrowserDownloader { }   // 重复了 "download" 语义

// 使用时冗余
use crate::download::{HttpDownloader, BrowserDownloader};
let http = HttpDownloader::new();
```

### 适用场景

- `scheduler::Memory` 而非 `scheduler::MemoryScheduler`
- `download::Http` 而非 `download::HttpDownloader`
- `download::Browser` 而非 `download::BrowserDownloader`
- `middleware::RateLimit` 而非 `middleware::RateLimitMiddleware`

### Builder 模式方法

所有 builder 模式的方法统一使用 `with_` 前缀：

```rust
impl Settings {
    pub fn with_timeout(mut self, timeout: Duration) -> Self { }
    pub fn with_retry_times(mut self, times: u32) -> Self { }
}

// 使用
let settings = Settings::default()
    .with_timeout(Duration::from_secs(30))
    .with_retry_times(3);
```

### 配置结构体

配置相关的结构体使用 `Config` 后缀：

```rust
pub struct HttpConfig { }
pub struct RuntimeConfig { }
```

## 可扩展性原则

### 集中配置

所有运行时配置应集中在 `Settings` 中，而不是分散在各个组件的构造函数中：

```rust
// ✅ 推荐：配置集中管理
let settings = Settings::default()
    .with_connection_pool_size(100)
    .with_timeout(Duration::from_secs(30));

// ❌ 不推荐：配置分散
let http = Http::new(100, Duration::from_secs(30));
```

### 组件解耦

组件应该可以独立配置，但也支持从全局配置初始化：

```rust
// 支持独立配置
let http = Http::with_config(HttpConfig { pool_size: 100 });

// 也支持 builder 模式
let http = Http::new().with_pool_size(100);
```

## 文档注释

所有公开 API 必须有文档注释，说明用途和示例。
