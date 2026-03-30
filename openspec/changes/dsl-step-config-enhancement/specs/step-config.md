# Spec: DSL Step 配置增强

## 1. Meta 配置

### 1.1 Schema

```rust
pub struct StepConfig {
    // ... 现有字段
    pub meta: Option<BTreeMap<String, Value>>,
}
```

### 1.2 JSON 格式

```json
{
  "id": "parse_list",
  "type": "node",
  "meta": {
    "source": "homepage",
    "category": "electronics"
  }
}
```

### 1.3 行为

- meta 字段在生成 request 时合并到 request.meta
- 与 parsed_fields 一起透传到下一个 step
- 可在 dedup.key 中引用（如 `meta.category`）

---

## 2. Dedup 配置

### 2.1 Schema

```rust
pub struct DedupConfig {
    pub enabled: bool,
    pub key: Vec<String>,
    pub ttl: u64,
    pub scope: String,
    pub namespace: Option<String>,
}
```

### 2.2 JSON 格式

```json
{
  "id": "parse_detail",
  "type": "end",
  "dedup": {
    "enabled": true,
    "key": ["sku_id", "meta.shop_id"],
    "ttl": 604800,
    "scope": "TASK",
    "namespace": "product"
  }
}
```

### 2.3 行为

- `key` 从 parsed_fields 和 meta 中提取值
- 按 `|` 拼接生成 dedup_value
- `scope`:
  - `TASK`: `dedup_value = join("|", values)`
  - `STEP`: `dedup_value = "step=" + step.id + "|" + join("|", values)`
  - `CUSTOM`: `dedup_value = "ns=" + namespace + "|" + join("|", values)`
- 任一 key 值缺失：DROP(invalid_dedup_key)
- 已存在：DROP(dedup_hit)


---

## 3. Retry 配置

### 3.1 Schema

```rust
pub struct RetryConfig {
    pub count: u32,
    pub http_status: Vec<u16>,
    pub backoff: Vec<u64>,
}
```

### 3.2 JSON 格式

```json
{
  "id": "parse_detail",
  "type": "end",
  "retry": {
    "count": 3,
    "http_status": [500, 502, 503, 504],
    "backoff": [1000, 2000, 5000]
  }
}
```

### 3.3 行为

- 请求失败时检查 http_status 是否匹配
- 匹配则重试，最多 count 次
- 使用 backoff[attempt] 延迟（ms）
- backoff 不足时使用最后一个值


---

## 4. Schedule 配置

### 4.1 Schema

```rust
pub struct ScheduleConfig {
    pub concurrency: Option<u32>,
    pub interval: Option<u64>,
}
```

### 4.2 JSON 格式

```json
{
  "id": "parse_list",
  "type": "node",
  "schedule": {
    "concurrency": 4,
    "interval": 1000
  }
}
```

### 4.3 行为

- `concurrency`: 限制该 step 的并发请求数
- `interval`: 同一 step 的请求最小间隔（ms）
