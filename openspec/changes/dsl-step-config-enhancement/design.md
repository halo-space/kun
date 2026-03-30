# Design: DSL Step 配置增强

## 架构概览

在现有 DSL 架构基础上，扩展 StepConfig 结构以支持更多配置项。

## 数据结构变更

### 1. StepConfig 扩展

```rust
pub struct StepConfig {
    pub id: String,
    pub r#type: Option<String>,
    pub callback: Option<String>,
    pub fetch: FetchConfig,
    pub parse: ParseConfig,
    pub validate: Vec<BTreeMap<String, Value>>,
    pub route: BTreeMap<String, Value>,
    pub output: BTreeMap<String, Value>,
    pub runtime: BTreeMap<String, Value>,
    pub middlewares: BTreeMap<String, Value>,

    // 新增字段
    pub meta: Option<BTreeMap<String, Value>>,
    pub dedup: Option<DedupConfig>,
    pub schedule: Option<ScheduleConfig>,
    pub retry: Option<RetryConfig>,
}
```

### 2. DedupConfig

```rust
pub struct DedupConfig {
    pub enabled: bool,
    pub key: Vec<String>,
    pub ttl: u64,
    pub scope: String,  // "TASK" | "STEP" | "CUSTOM"
    pub namespace: Option<String>,
}
```

### 3. ScheduleConfig

```rust
pub struct ScheduleConfig {
    pub concurrency: Option<u32>,
    pub interval: Option<u64>,  // ms
}
```

### 4. RetryConfig

```rust
pub struct RetryConfig {
    pub count: u32,
    pub http_status: Vec<u16>,
    pub backoff: Vec<u64>,  // ms
}
```

## 实现策略

### 阶段 1：Schema 扩展
1. 在 `src/rules/schema.rs` 中添加新结构体
2. 更新 StepConfig 添加新字段
3. 实现 Default trait

### 阶段 2：解析支持
1. 在 `src/rules/load.rs` 中添加解析逻辑
2. 支持从 JSON 反序列化新配置

### 阶段 3：编译支持
1. 在 `src/rules/compile.rs` 中处理新配置
2. 将配置编译到 CompiledStep

### 阶段 4：运行时应用
1. 在 `src/rules/run.rs` 中应用配置
2. dedup: 在请求前检查去重
3. retry: 在失败时重试
4. schedule: 控制并发和间隔

