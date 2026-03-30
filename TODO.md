# TODO

这里只保留当前仍然成立、并且会影响后续实现取舍的能力缺口。
更细的任务拆分以 `openspec/changes/track-kun-core-capability-gaps/tasks.md` 为准。

## 1. Parser 缺口

- HTML XPath 仍然暂缓。当前 XPath 基于 XML 解析器，在不规范 HTML 上不稳定，HTML 场景继续建议优先使用 CSS。
- `ocr` 相关解析能力仍未实现，当前不纳入落地范围。
- parse 后处理还比较薄：多选择器兜底、normalize、类型转换等能力还没有统一抽象。

## 2. Validation 缺口

- 共享 validation 目前已经支持 `type` 与 `rule.required`。
- `regex`、`min/max`、`enum` 等规则仍未进入共享 validation 能力。

## 3. Request / Browser / Middleware 缺口

- 代码爬虫侧的 request 能力已经比较完整，但 DSL 到共享 request 模型的映射还没有完全收敛。
- browser 路线当前只落了最小可用能力；`stealth`、`fingerprint_profile`、browser `session`、非 `GET` browser request、request body 仍未实现。
- proxy / cookies 已接到真实 HTTP 下载链路，但更细的 DSL 配置面和高级策略还没有统一。

## 4. Pipeline 与输出能力

- 当前只有一条 `pipeline` item 处理链路，不再维护独立的 `output.sinks` 概念。
- 内置输出能力目前已有 `pipeline::Memory` 与 `pipeline::JsonLines`。
- 文件、数据库、消息队列等 pipeline 仍待按模块逐步补齐。

## 5. Plugin 边界

- `Engine::load_plugins()` 当前只支持 `middleware` kind 的自动装载。
- `rules`、`provider`、`storage` 目前只是已命名的 manifest kind，不代表底层 runtime 已经具备对应装配能力。
- 如果后面要扩展新的 plugin kind，必须先有清晰的底层 owner：例如稳定的 pipeline/storage runtime、provider 抽象或其它可验证的共享链路。

## 6. DSL 对齐缺口

- DSL 配置面仍然整体后置，优先跟随代码爬虫与共享底层能力收敛。
- `step.meta`、`links[].meta`、dedup / schedule / retry、以及 step validate 已经落到底层能力。
- `next_url_config` 已支持 `FIELD`、`TEMPLATE`、`JOIN`、`FUNCTION`，其中最小函数集为 `concat`、`replace`、`coalesce`。
- 日期/时间表达式、更多函数、以及更完整的 request / parse 配置还没有统一设计。

## 7. API 命名与导出

- 评估 `item::Item` 这个公开路径是否需要继续收口成更顺手的对外导出，同时保持与 `request::Request`、`response::Response`、`settings::Settings` 的公开 API 一致性。

## 8. Engine 结构缺口

- `engine` 当前主循环和任务执行逻辑已经拆分出独立子模块；如果后面继续增长，再评估是否把调度主循环、runtime 组装等职责继续细分。
