# Examples

当前 `examples/` 先只保留围绕 kun 实际能力的示例，并统一使用
`https://ep.shxwcb.com/2026/03/period.xml` 作为入口场景。

- `period_xml_spider.rs`：代码爬虫的多级回调、`meta` 透传、`follow` 链路
- `concurrency_control.rs`：从 `period.xml` 扇出多个版面请求，观察并发与节流配置
- `custom_middleware.rs`：引擎级中间件注册与执行顺序
- `plugins_demo.rs`：插件清单加载、插件注册冲突规则、插件式中间件接入
- `ai_extraction.rs`：AI selector 直接处理 `period.xml`

DSL 示例先不继续堆叠在这里。等 DSL 配置面和模块边界重新收敛后，再按模块逐步补回示例。
