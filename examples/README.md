# Examples

当前 `examples/` 先只保留围绕 kun 实际能力的示例，并统一使用
`https://ep.shxwcb.com/2026/03/period.xml` 作为入口场景。

除非示例要演示自定义底层组件，否则统一从 `Engine::new()` 起步。
它默认就是 `scheduler::Memory + download::Http + download::Browser + store::File`。

- `period_xml_spider.rs`：代码爬虫的多级回调、`meta` 透传、`follow` 链路
- `memory.rs`：`parse -> item -> pipeline -> store::Memory`，展示最终 item 直接在 `parse()` 里组装
- `file.rs`：`parse -> item -> pipeline -> store::File`，并演示 pretty file format
- `sqlite.rs`：`parse -> item -> pipeline -> store::Sqlite`
- `webhook.rs`：`parse -> item -> pipeline -> store::Webhook`，并演示 retry/backoff
- `redis.rs`：`parse -> item -> pipeline -> store::Redis`
- `elasticsearch.rs`：自定义 `Store` trait，实现 Elasticsearch `_doc / _bulk` 写入
- `kafka.rs`：`parse -> item -> pipeline -> store::Kafka`，并演示 message key / headers
- `concurrency_control.rs`：从 `period.xml` 扇出多个版面请求，观察并发与节流配置
- `http_cache.rs`：`http_cache` 的 `ttl / strategy / file backend / stats` 最小用法示例
- `custom_http_cache.rs`：自定义 `middleware::http_cache::Cache` backend，再复用内置 `HttpCache` 行为
- `robots_site_policy.rs`：`robots::Site::pattern / host / origin` 的 precedence、delay merge 和 sitemap union
- `custom_middleware.rs`：引擎级中间件注册与执行顺序
- `middleware_plugin.rs`：最小 middleware plugin 装配示例，展示 manifest / registry / factory / `load_plugins()`
- `custom_dedup.rs`：自定义 `dedup::Dedup`，替换默认 request 去重实现
- `custom_scheduler.rs`：自定义 `scheduler::Scheduler` 与自定义 `scheduler::checkpoint::Persist` 的接线方式
- `ai_extraction.rs`：AI selector 直接处理 `period.xml`

如果你要接 PostgreSQL，当前也建议按 `custom Store` 模式自己实现，而不是继续加内置 PG 分支。

DSL 示例暂不放在 `examples/`；等配置面和模块边界收敛后，再按模块逐步补回。
