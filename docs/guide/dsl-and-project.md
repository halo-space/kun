# DSL 与项目协作

[返回使用手册](../guide.md)

## DSL 状态

项目里仍保留 `Spider::rules()` 这条入口，引擎也会继续负责加载和编译 rules：

```rust
use halo_spider::rules::Config as RulesConfig;
use halo_spider::spider::Spider;

struct MyDslSpider;

impl Spider for MyDslSpider {
    fn name(&self) -> &str {
        "my_dsl_spider"
    }

    fn start_urls(&self) -> Vec<String> {
        vec!["https://example.com".to_string()]
    }

    fn rules(&self) -> Option<RulesConfig> {
        Some(RulesConfig::local("path/to/rules.json"))
    }
}
```

引擎会自动加载、编译规则并分发到 DSL step。

但当前不把 DSL 作为主推使用方式，`examples/` 也不再维护字段级 DSL 示例。原因很简单：

- 当前优先把代码爬虫与共享底层能力做实
- DSL 只是这些底层能力的配置化入口，不是另一套独立运行时
- 新一版 DSL 设计里，这些共享能力会统一收口到 `engine(...)` 配置入口，和 `Engine::with_dedup(...)` 这类代码能力命名保持一致
- 在配置面重新收敛前，不继续维护容易过期的 DSL 字段清单和外部示例

DSL 当前的定位可以先简单理解成：把代码爬虫已有底层能力做成配置化入口。它不是另一套独立运行时。无论是 Rust 回调模式还是 JSON DSL 模式，底层都走同一套框架执行链路：

```text
Spider / rules
    -> Request
    -> Engine
    -> Downloader
    -> Response
    -> callback 或 DSL step
    -> Output(items / requests)
    -> Engine 继续调度
```

这意味着：

- `Request` 是统一的执行单元，DSL 生成的请求和代码里手写的请求没有本质区别
- `step.fetch.request` / `step.fetch.browser` 也是往同一个 `Request` 模型上做显式覆盖
- 代码 Spider 现在可以通过 `Spider::validator()` 显式启用共享 validation；item 产出会继续走统一的 `pipeline -> validator -> store` 链路
- `meta` 是请求级上下文参数，用来携带当前请求和后续链路需要透传的数据
- `Response.body` 保留原始响应字节，`Response.text` 是从 `body` 派生出的解码文本
- 去重、调度、限流、重试等能力属于框架本身的 engine 能力；在 DSL v1 里会统一映射到 `engine.dedup / engine.schedule / engine.limits / engine.retry`

如果你现在要写新的爬虫，优先建议直接用代码模式；等共享底层能力进一步稳定后，再回到 DSL 配置面做统一收口。

当前这轮重新讨论后的 DSL v1 设计稿已经单独整理在这里：

- [Rules DSL 设计（v1）](./rules-dsl.md)

这份文档更偏“配置模型和字段设计”，适合在需要统一 DSL 结构、补充字段说明、继续讨论配置边界时查看。

## 项目协作

项目当前用下面几层来组织协作：

- `README.md`：项目首页与导航
- `docs/guide.md`：使用手册入口
- `docs/capabilities.md`：模块能力与边界
- `docs/operations.md`：运维 / 观测 / 控制说明
- `openspec/specs/`：规范源
- `openspec/changes/`：需求、方案、任务的变更入口

如果你想参与开发或了解项目流程，请查看 [CONTRIBUTING.md](../../CONTRIBUTING.md)。

## License

MIT
