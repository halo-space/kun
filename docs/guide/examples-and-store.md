# 示例、Pipeline 与 Store

[返回使用手册](../guide.md)

## 示例

```bash
# 基础能力示例（统一使用 period.xml 场景）
cargo run --example period_xml_spider
cargo run --example memory
cargo run --example file
cargo run --example sqlite
cargo run --example webhook
cargo run --example redis
cargo run --example robots_site_policy
HALO_SPIDER_ES_URL=http://127.0.0.1:9200 cargo run --example elasticsearch
HALO_SPIDER_KAFKA_BROKERS=127.0.0.1:9092 cargo run --example kafka
cargo run --example custom_dedup
cargo run --example custom_middleware
cargo run --example middleware_plugin
cargo run --example custom_scheduler
cargo run --example telemetry

# AI 选择器示例（需要 OPENAI_API_KEY 环境变量）
cargo run --example ai_extraction --features ai-selector

# 并发控制示例
cargo run --example concurrency_control
```

如果只是想知道每个示例大概演示什么，先看 [examples/README.md](../../examples/README.md)。

## Pipeline 与 Store

当前 item 执行链路固定为：

```text
parse -> item -> pipeline -> store
```

其中：

- `pipeline` 只负责 item 处理与过滤，例如 normalize、补默认值、丢弃无效 item
- `store` 负责最终持久化或投递，例如文件、数据库、HTTP API、消息队列

Engine 现在保留显式 `with_dedup(...)`、`with_robots(...)`、`with_pipeline(...)` 与 `with_store(...)` 这些组件插槽。
当前不再推荐 `with_pipeline((A, B))` 这类元组组合写法；如果确实需要多个 item 处理步骤，直接在你自己的 pipeline 类型里按顺序组合即可。

如果没有显式调用 `with_store(...)`，引擎默认使用 `store::File::default()`，并把结果写到 `output/<spider_name>.jsonl`。

需要跨请求透传上下文时，优先把数据放进 `request.meta`，并在最后一个 `parse()` / callback 里组装最终 item，而不是让 pipeline/store 充当隐藏状态通道。

如果你什么都不配，直接：

```rust
let engine = Engine::new();
```

那默认就是：

- `scheduler::Memory`
- `download::Http`
- `download::Browser`
- `dedup::Memory`
- `robots::Memory`
- `store::File::default()`，输出到 `output/<spider_name>.jsonl`

关于 dedup 默认值，这里也明确一下：

- `Engine::new()` 继续默认用精确 `dedup::Memory`
- `dedup::Bloom` 是显式 opt-in，不默认替换
- 原因是默认行为优先保 correctness，不默认引入布隆误判导致的潜在漏抓

## Store 批量语义

`Store` 当前同时暴露 `write()` 和 `batch_write()` 两个入口：

- `write()` 负责单条 item 的最终写入或投递
- `batch_write()` 负责一批 item 的最终写入或投递
- engine 会把同一次 `parse()` / callback 输出里经过 pipeline 保留的 items 收成一批，并优先调用 `store.batch_write(...)`
- 默认 `Store::batch_write()` 会退回逐条调用 `write()`，所以最简单的 store 只实现单条写入也能正常工作
- 如果某个 store 底层支持原生批量写入，它可以覆盖 `batch_write()` 来减少锁竞争、文件打开次数或数据库往返次数

例如：

```rust
use halo_spider::store::{File, Sqlite};

let mut engine = Engine::from_parts(scheduler, http, browser)
    .with_store(Sqlite::new("output/items.db"));

let mut engine = Engine::from_parts(scheduler, http, browser)
    .with_store(File::new("output/items.jsonl"));
```

如果你需要 item 预处理，再额外调用 `with_pipeline(...)` 即可。

## 内置 Store

当前内置 `store` 有：

- `store::Memory`
- `store::File`
- `store::Sqlite`
- `store::Webhook`
- `store::Redis`
- `store::Kafka`

当前 `store::File` 的最小增强语义是：

- 默认仍然写紧凑的 JSON Lines
- 可以通过 `with_format(store::FileFormat::PrettyJsonBlocks)` 切到更适合人工查看的 pretty block 形式
- 可以通过 `with_rotate_items(...)` 或 `with_rotate_bytes(...)` 把输出切分成编号文件，例如 `items-0001.jsonl`
- 这些增强仍然只发生在同一个 `store::File` 边界上，不引入第二套文件输出 runtime

当前 SQLite store 的最小语义是：

- `open()` 只负责建库建表，不会自动清空旧数据
- 每条 item 都会保留一份完整 `item_json`
- 显式映射的字段列按声明类型写入；缺失字段写 `NULL`
- 如果字段值类型和列类型不匹配，会返回显式 store error，而不是静默转换

当前 Webhook store 的最小语义是：

- 把完整 item JSON 通过 `POST` 或 `PUT` 推送到目标 HTTP endpoint
- 支持追加固定请求头
- 支持 `with_retry_limit(...)` 与 `with_retry_backoff(...)`
- 当前只对请求错误和 `429 / 5xx` 做重试；其它非 `2xx` 仍然直接报错
- 如果接口返回非 `2xx`，会返回显式 store error，而不是静默忽略失败

当前 Redis store 的最小语义是：

- 支持 `redis://` 连接 URL，并接住最小 `AUTH` / `SELECT` 语义
- `Redis::new(...)` 直接把完整 item JSON 用 `SADD` 写入目标 set
- `batch_write()` 会把一批 item JSON 合并成同一个 `SADD key value...` 命令
- 当前明确不做另一套消息输出 runtime；Redis 仍然只是同一条 `store` 边界上的一个内置实现

当前 Kafka store 的最小语义是：

- `Kafka::new(brokers, topic)` 把完整 item JSON 作为消息 value 发到指定 topic
- 支持 `with_key(...)` / `with_key_field(...)`
- 支持 `with_header(...)` / `with_header_field(...)`
- `batch_write()` 会在同一次 store 调用里连续发送多条 item JSON 消息
- 如果 Kafka producer 返回投递错误，store 返回显式 store error
- 当前仍不支持显式 partition、事务、schema registry 或 consumer/group 这类更高阶 Kafka 语义

## 自定义 Store

自定义 store 也走同一条主链。只要实现 `Store` trait，再通过 `Engine::with_store(...)` 挂进去即可。

例如，用户自己的 Elasticsearch / PostgreSQL store 都可以这样接入：

```rust
use halo_spider::engine::Engine;
use halo_spider::error::SpiderError;
use halo_spider::item::Item;
use halo_spider::store::Store;

#[derive(Clone)]
struct ElasticsearchStore {
    client: reqwest::Client,
    base_url: String,
    index: String,
}

impl ElasticsearchStore {
    fn new(base_url: impl Into<String>, index: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            index: index.into(),
        }
    }
}

impl Store for ElasticsearchStore {
    async fn write(&self, item: &Item, _spider_name: &str) -> Result<(), SpiderError> {
        self.client
            .post(format!("{}/{}/_doc", self.base_url.trim_end_matches('/'), self.index))
            .json(&item.to_json())
            .send()
            .await
            .map_err(|error| SpiderError::engine(format!("elasticsearch store request failed: {error}")))?;
        Ok(())
    }
}

let mut engine = Engine::from_parts(scheduler, http, browser)
    .with_store(ElasticsearchStore::new("http://127.0.0.1:9200", "period_items"));
```

如果你的自定义 store 底层本身支持批量写入，比如 Elasticsearch `_bulk`、ClickHouse 批量 insert、对象存储批量上传，也推荐一起覆盖 `batch_write()`。

当前内置维护范围也明确一下：

- 框架内置继续维护 `Memory / File / Sqlite / Webhook / Redis / Kafka`
- 更专门的数据库、对象存储、第三方 API、复杂 MQ 语义，优先继续通过用户自定义 `Store` 扩展

后续更多文件格式与更完整消息语义也继续扩展在 `store` 这一层，而不是再拆新的输出运行时。
