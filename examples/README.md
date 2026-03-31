# Examples

当前 `examples/` 先只保留围绕 kun 实际能力的示例，并统一使用
`https://ep.shxwcb.com/2026/03/period.xml` 作为入口场景。

- `period_xml_spider.rs`：代码爬虫的多级回调、`meta` 透传、`follow` 链路
- `pipeline_memory.rs`：单一 `pipeline.process()` 链路、自定义 pipeline 与 `pipeline::Memory`
- `pipeline_json_lines.rs`：单一 `pipeline.process()` 链路、自定义 pipeline 与 `pipeline::JsonLines`
- `concurrency_control.rs`：从 `period.xml` 扇出多个版面请求，观察并发与节流配置
- `custom_middleware.rs`：引擎级中间件注册与执行顺序
- `plugins_demo.rs`：当前 middleware 插件接入路径、`PluginRegistry` 的 `(kind, name)` 唯一性与 override 规则
- `ai_extraction.rs`：AI selector 直接处理 `period.xml`

DSL 示例暂不放在 `examples/`；等配置面和模块边界收敛后，再按模块逐步补回。
