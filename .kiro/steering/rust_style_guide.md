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

### 异步优先

所有涉及能力执行、I/O、运行时调度或可能阻塞链路的功能，默认都要设计为异步接口。

#### 适用范围

- downloader 的 `fetch`
- spider / callback 的解析入口
- pipeline 的 `open / process / close`
- middleware、plugin、runtime 组装后真正参与执行的 hook
- 后续新增的存储、浏览器、网络、AI、调度恢复等能力接口

#### 原则

- 能力接口默认优先 `async`
- 不要先做同步版本，再在外层包一层 `spawn_blocking` 或临时桥接
- 如果是纯数据构造、builder、getter、纯值转换这类不涉及等待或阻塞的辅助方法，保持同步即可
- 如果某个能力暂时只能同步实现，需要在设计或文档里明确说明边界，而不是默认把同步接口扩散成公开约定

#### ✅ 推荐

```rust
#[async_trait::async_trait]
pub trait Pipeline {
    async fn process(&self, item: &mut Item, spider_name: &str) -> Result<bool, SpiderError>;
}
```

#### ❌ 不推荐

```rust
pub trait Pipeline {
    fn process(&self, item: &mut Item, spider_name: &str) -> Result<bool, SpiderError>;
}
```

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
